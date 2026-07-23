//! SQLite bootstrap. Schema source of truth: Documents/05_DATA_MODEL.md.
//! Vector tables (sqlite-vec) are created at runtime in Phase 2 with dims
//! taken from `embedding_config`; this module owns everything else.

use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 5;

/// Registers sqlite-vec as an auto-extension. MUST run before any connection
/// opens (the app calls it once at startup; benches/tests call it themselves).
pub fn register_vec_extension() {
    type ExtInit = unsafe extern "C" fn(
        *mut rusqlite::ffi::sqlite3,
        *mut *mut std::os::raw::c_char,
        *const rusqlite::ffi::sqlite3_api_routines,
    ) -> std::os::raw::c_int;
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<*const (), ExtInit>(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
  id            TEXT PRIMARY KEY,
  title         TEXT NOT NULL,
  kind          TEXT NOT NULL,
  status        TEXT NOT NULL,
  language      TEXT NOT NULL,
  context_url   TEXT,
  created_at    INTEGER NOT NULL,
  started_at    INTEGER,
  closed_at     INTEGER,
  notes         TEXT
);

CREATE TABLE IF NOT EXISTS cards (
  id            TEXT PRIMARY KEY,
  session_id    TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  title         TEXT NOT NULL,
  tags          TEXT,
  language      TEXT NOT NULL,
  source        TEXT NOT NULL,
  created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS bullets (
  id            TEXT PRIMARY KEY,
  card_id       TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  position      INTEGER NOT NULL,
  text          TEXT NOT NULL,          -- canonical (default length)
  text_short    TEXT,                   -- 1-2 word variant (adapt job)
  text_long     TEXT,                   -- fuller variant (adapt job)
  provenance    TEXT NOT NULL DEFAULT 'prepared'
);

CREATE TABLE IF NOT EXISTS embedding_config (
  id            INTEGER PRIMARY KEY CHECK (id = 1),
  model         TEXT NOT NULL,
  dims          INTEGER NOT NULL,
  created_at    INTEGER NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS card_fts USING fts5(
  card_id UNINDEXED, title, bullets_text
);

CREATE TABLE IF NOT EXISTS transcript (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  speaker       TEXT NOT NULL,
  ts_ms         INTEGER NOT NULL,
  text          TEXT NOT NULL,
  language      TEXT
);

CREATE TABLE IF NOT EXISTS coverage (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  card_id       TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  bullet_id     TEXT REFERENCES bullets(id) ON DELETE CASCADE,
  covered       INTEGER NOT NULL,
  score         REAL,
  ts_ms         INTEGER
);

CREATE TABLE IF NOT EXISTS card_events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  card_id       TEXT NOT NULL,
  ts_ms         INTEGER NOT NULL,
  mode          TEXT NOT NULL,
  score         REAL,
  runner_up     TEXT,
  runner_score  REAL,
  fused_rank    TEXT
);

CREATE TABLE IF NOT EXISTS context_research (
  session_id    TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  raw_text      TEXT,
  summary       TEXT,
  fetched_at    INTEGER
);

CREATE TABLE IF NOT EXISTS settings (
  key           TEXT PRIMARY KEY,
  value         TEXT NOT NULL
);

-- FTS and vec tables are virtual: FK cascades never touch them, so derived
-- rows are cleaned up by triggers. bullet_vec cleanup rides on bullet deletes
-- (which themselves cascade from cards).
CREATE TRIGGER IF NOT EXISTS trg_cards_delete AFTER DELETE ON cards BEGIN
  DELETE FROM card_fts  WHERE card_id  = OLD.id;
  DELETE FROM card_vec  WHERE card_id  = OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS trg_bullets_delete AFTER DELETE ON bullets BEGIN
  DELETE FROM bullet_vec WHERE bullet_id = OLD.id;
END;
"#;

pub fn open_and_migrate(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let user_version: i64 =
        conn.query_row("SELECT * FROM pragma_user_version", [], |r| r.get(0))?;
    if user_version < 2 {
        // v1 shipped a contentless card_fts; v2 makes it contentful with card_id.
        conn.execute_batch("DROP TABLE IF EXISTS card_fts;")?;
    }

    conn.execute_batch(SCHEMA)?;

    if user_version > 0 && user_version < 3 {
        // v3: card_fts may have been recreated empty on the v1→v2 drop, and the
        // delete triggers arrive only now — rebuild the keyword index from truth.
        conn.execute_batch(
            "DELETE FROM card_fts;
             INSERT INTO card_fts (card_id, title, bullets_text)
               SELECT c.id, c.title,
                      COALESCE((SELECT group_concat(text, ' ' ORDER BY position)
                                FROM bullets WHERE card_id = c.id), '')
               FROM cards c;",
        )?;
    }

    if user_version > 0 && user_version < 4 {
        // v4: per-bullet length variants (owner decision 2026-07-23 — the
        // bullet-length setting restyles the WHOLE corpus instantly, so the
        // variants are stored, not regenerated). Base `text` stays canonical.
        // Column-existence guard: a crash between the two ALTERs must not
        // brick the next startup with "duplicate column" (audit 2026-07-23).
        let has_col = |col: &str| -> rusqlite::Result<bool> {
            let mut stmt = conn.prepare("SELECT 1 FROM pragma_table_info('bullets') WHERE name = ?1")?;
            stmt.exists([col])
        };
        conn.execute_batch("BEGIN")?;
        if !has_col("text_short")? {
            conn.execute_batch("ALTER TABLE bullets ADD COLUMN text_short TEXT;")?;
        }
        if !has_col("text_long")? {
            conn.execute_batch("ALTER TABLE bullets ADD COLUMN text_long TEXT;")?;
        }
        conn.execute_batch("COMMIT")?;
    }

    // v5: coverage FKs lacked ON DELETE CASCADE, so any covered bullet made
    // its card permanently undeletable (audit 2026-07-23, verified by repro).
    // Gate on the ACTUAL schema, not user_version — self-heals any DB whose
    // version got bumped without the rebuild. SQLite cannot alter
    // constraints — rebuild the table.
    let coverage_lacks_cascade: bool = user_version > 0 && {
        let mut stmt = conn.prepare(
            "SELECT 1 FROM pragma_foreign_key_list('coverage')
             WHERE \"table\" = 'cards' AND on_delete <> 'CASCADE'",
        )?;
        stmt.exists([])?
    };
    if coverage_lacks_cascade {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE coverage_v5 (
               id            INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
               card_id       TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
               bullet_id     TEXT REFERENCES bullets(id) ON DELETE CASCADE,
               covered       INTEGER NOT NULL,
               score         REAL,
               ts_ms         INTEGER
             );
             INSERT INTO coverage_v5 (id, session_id, card_id, bullet_id, covered, score, ts_ms)
               SELECT id, session_id, card_id, bullet_id, covered, score, ts_ms FROM coverage;
             DROP TABLE coverage;
             ALTER TABLE coverage_v5 RENAME TO coverage;
             COMMIT;",
        )?;
    }

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(conn)
}

/// Boot-time guard: the stored embedding space must match the compiled one.
/// Empty corpus → adopt the compiled config (recreate vec tables if needed).
/// Non-empty corpus + mismatch → hard error; silent mixing of embedding
/// spaces corrupts retrieval invisibly.
pub fn check_embedding_compat(
    conn: &Connection,
    model: &str,
    dims: i64,
) -> Result<(), String> {
    let stored: Option<(String, i64)> = conn
        .query_row(
            "SELECT model, dims FROM embedding_config WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .unwrap_or(None);
    let Some((s_model, s_dims)) = stored else { return Ok(()) };
    if s_model == model && s_dims == dims {
        return Ok(());
    }
    let cards: i64 = conn
        .query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if cards == 0 {
        conn.execute_batch(&format!(
            "DELETE FROM embedding_config;
             INSERT INTO embedding_config (id, model, dims, created_at)
               VALUES (1, '{model}', {dims}, strftime('%s','now'));
             DROP TABLE IF EXISTS card_vec;
             DROP TABLE IF EXISTS bullet_vec;"
        ))
        .map_err(|e| e.to_string())?;
        ensure_vec_tables(conn, dims).map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err(format!(
        "embedding model changed ({s_model}/{s_dims}d stored vs {model}/{dims}d compiled) \
         with a non-empty corpus; re-import the corpus (wipe + import) to re-embed"
    ))
}

/// Embedding model registration. One row; changing it means re-embedding.
pub fn ensure_embedding_config(
    conn: &Connection,
    model: &str,
    dims: i64,
) -> rusqlite::Result<(String, i64)> {
    conn.execute(
        "INSERT OR IGNORE INTO embedding_config (id, model, dims, created_at)
         VALUES (1, ?1, ?2, strftime('%s','now'))",
        rusqlite::params![model, dims],
    )?;
    conn.query_row(
        "SELECT model, dims FROM embedding_config WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// vec0 tables are created here (not in SCHEMA) because their dimension
/// comes from embedding_config.
pub fn ensure_vec_tables(conn: &Connection, dims: i64) -> rusqlite::Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS card_vec
           USING vec0(card_id TEXT PRIMARY KEY, embedding float[{dims}]);
         CREATE VIRTUAL TABLE IF NOT EXISTS bullet_vec
           USING vec0(bullet_id TEXT PRIMARY KEY, embedding float[{dims}]);"
    ))
}

pub fn table_names(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type IN ('table','virtual table') \
         AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'card_fts_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}
