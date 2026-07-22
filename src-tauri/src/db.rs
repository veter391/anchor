//! SQLite bootstrap. Schema source of truth: Documents/05_DATA_MODEL.md.
//! Vector tables (sqlite-vec) are created at runtime in Phase 2 with dims
//! taken from `embedding_config`; this module owns everything else.

use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 2;

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
  text          TEXT NOT NULL,
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
  card_id       TEXT NOT NULL REFERENCES cards(id),
  bullet_id     TEXT REFERENCES bullets(id),
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
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(conn)
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
