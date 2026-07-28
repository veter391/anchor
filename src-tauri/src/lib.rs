// Anchor — live notes overlay. Prepared, not prompted.
// Phase 2 (+review fixes): corpus import, local embeddings, hybrid retrieval.

pub mod audio;
pub mod cards;
pub mod db;
pub mod embed;
pub mod ingest;
pub mod live;
pub mod matcher;
pub mod mode2;
pub mod overlay_input;
pub mod paths;
pub mod preflight;
pub mod search;
pub mod store;
pub mod textfmt;

use embed::Embedder;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

pub struct Db {
    pub(crate) conn: Mutex<Connection>,
    path: PathBuf,
}

#[derive(serde::Serialize)]
struct BootInfo {
    db_path: String,
    schema_version: i64,
    tables: Vec<String>,
    embedding_model: String,
    embedding_dims: i64,
    embedder_loaded: bool,
    cards: usize,
}

#[tauri::command]
fn boot_info(
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
) -> Result<BootInfo, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let tables = db::table_names(&conn).map_err(|e| e.to_string())?;
    let (model, dims) = db::ensure_embedding_config(&conn, embed::MODEL_ID, embed::DIMS as i64)
        .map_err(|e| e.to_string())?;
    let cards: i64 = conn
        .query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(BootInfo {
        db_path: db.path.display().to_string(),
        schema_version: db::SCHEMA_VERSION,
        tables,
        embedding_model: model,
        embedding_dims: dims,
        embedder_loaded: embedder.is_loaded(),
        cards: cards as usize,
    })
}

/// Shared import path: parse + embed happen BEFORE the DB lock is taken —
/// model inference is the slow part and must never serialize other commands.
fn import_markdown(
    db: &Db,
    embedder: &Embedder,
    markdown: &str,
    default_lang: &str,
    session_id: Option<&str>,
) -> Result<store::ImportReport, String> {
    let parsed = cards::parse_markdown(markdown, default_lang);
    let vectors = store::embed_import(embedder, &parsed)?;
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::write_import(&mut conn, parsed, vectors, session_id, "prepared")
}

#[tauri::command]
fn import_cards(
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
    markdown: String,
    default_lang: Option<String>,
    session_id: Option<String>,
) -> Result<store::ImportReport, String> {
    import_markdown(
        &db,
        &embedder,
        &markdown,
        default_lang.as_deref().unwrap_or("en"),
        session_id.as_deref(),
    )
}

#[tauri::command]
fn list_session_cards(
    db: tauri::State<'_, Db>,
    session_id: String,
) -> Result<Vec<store::CardRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::list_session_cards(&conn, &session_id)
}

#[tauri::command]
fn add_library_cards_to_session(
    db: tauri::State<'_, Db>,
    session_id: String,
    card_ids: Vec<String>,
) -> Result<usize, String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::copy_cards(&mut conn, &card_ids, Some(&session_id))
}

/// Promote session-owned cards into the global library (a copy; the session
/// keeps its own). Lets a card built inside a call be reused across sessions.
#[tauri::command]
fn promote_cards_to_library(
    db: tauri::State<'_, Db>,
    card_ids: Vec<String>,
) -> Result<usize, String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::copy_cards(&mut conn, &card_ids, None)
}

/// A session's stored transcript (text only) for its detail/report view.
#[tauri::command]
fn session_transcript(
    db: tauri::State<'_, Db>,
    session_id: String,
) -> Result<Vec<store::TranscriptLine>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::session_transcript(&conn, &session_id)
}

/// Recursively reads every .md file under `path` and imports the lot.
#[tauri::command]
fn import_folder(
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
    path: String,
    default_lang: Option<String>,
    session_id: Option<String>,
) -> Result<store::ImportReport, String> {
    let mut files = Vec::new();
    collect_md_files(Path::new(&path), &mut files, 0)?;
    if files.is_empty() {
        return Err(format!("no .md files found under {path}"));
    }
    files.sort();
    let mut combined = String::new();
    for f in &files {
        let text = std::fs::read_to_string(f)
            .map_err(|e| format!("failed to read {}: {e}", f.display()))?;
        combined.push_str(&text);
        combined.push('\n');
    }
    import_markdown(
        &db,
        &embedder,
        &combined,
        default_lang.as_deref().unwrap_or("en"),
        session_id.as_deref(),
    )
}

fn collect_md_files(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) -> Result<(), String> {
    if depth > 4 {
        return Ok(()); // sane recursion cap for a notes folder
    }
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_md_files(&p, out, depth + 1)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(p);
        }
    }
    Ok(())
}

#[tauri::command]
fn list_cards(db: tauri::State<'_, Db>) -> Result<Vec<store::CardRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::list_cards(&conn)
}

#[tauri::command]
fn delete_card(db: tauri::State<'_, Db>, card_id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::delete_card(&conn, &card_id)
}

#[tauri::command]
fn wipe_corpus(db: tauri::State<'_, Db>) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::wipe_corpus(&conn)
}

#[derive(serde::Serialize)]
struct QueryResult {
    matches: Vec<search::Match>,
    top_card: Option<store::CardRow>,
}

/// Phase-2 "fake transcript": free text in, ranked cards out, top card
/// pushed to the overlay window. Embedding runs before the DB lock.
#[tauri::command]
fn query_cards(
    app: tauri::AppHandle,
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
    text: String,
) -> Result<QueryResult, String> {
    let qvec = search::embed_query_text(&embedder, &text)?;
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let matches = search::query_cards_with_vec(&conn, &qvec, &text)?;
    let top_card = match matches.first() {
        Some(m) => store::get_card(&conn, &m.card_id)?,
        None => None,
    };
    if let Some(card) = &top_card {
        app.emit_to("overlay", "card:show", card)
            .map_err(|e| e.to_string())?;
    }
    Ok(QueryResult { matches, top_card })
}

// ── Mode-2 config: local models + API providers ─────────────────────

#[derive(serde::Serialize)]
struct ModelRow {
    id: String,
    name: String,
    tagline: String,
    size_bytes: u64,
    licence: String,
    is_default: bool,
    installed: bool,
}

#[tauri::command]
fn list_models() -> Result<Vec<ModelRow>, String> {
    let app_data = paths::data_dir();
    Ok(mode2::models::REGISTRY
        .iter()
        .map(|m| ModelRow {
            id: m.id.into(),
            name: m.name.into(),
            tagline: m.tagline.into(),
            size_bytes: m.size_bytes,
            licence: m.licence.into(),
            is_default: m.is_default,
            installed: mode2::models::is_installed(&app_data, m),
        })
        .collect())
}

#[derive(serde::Serialize, Clone)]
struct DownloadProgress {
    id: String,
    downloaded: u64,
    total: u64,
}

#[tauri::command]
async fn download_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let info = mode2::models::find(&id).ok_or("unknown model id")?;
    let app_data = paths::data_dir();
    let app2 = app.clone();
    let id2 = id.clone();
    mode2::models::download(&app_data, info, move |downloaded, total| {
        app2.emit_to(
            "dashboard",
            "model:progress",
            DownloadProgress {
                id: id2.clone(),
                downloaded,
                total,
            },
        )
        .ok();
    })
    .await?;
    app.emit_to("dashboard", "model:done", &id).ok();
    Ok(())
}

#[tauri::command]
fn delete_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    // Registry-validate the id — never join raw frontend input into a path
    // (audit 2026-07-23: "../x" would escape the models dir).
    let info = mode2::models::find(&id).ok_or("unknown model id")?;
    let app_data = paths::data_dir();
    // If this model is the one loaded in RAM, drop it: keeping it would both
    // hold ~1-2 GB and let `ensure` serve a model whose file is gone.
    app.state::<Arc<mode2::local::LocalEngine>>().unload_if(info.id);
    mode2::models::delete(&app_data, info.id)
}

#[derive(serde::Serialize)]
struct LlmConfig {
    mode: String,
    local_model: String,
    api_provider: String,
    api_model: Option<String>,
    api_key_set: bool,
    bullet_style: String,
}

pub(crate) fn setting_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

fn setting_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        [key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The chosen speech model: "auto" (multilingual streaming, or the fallback if
/// missing), "multilingual" (EN/ES/RU/UK/DE, real-time), "english" (EN-only,
/// fastest), or "offline" (Parakeet + LocalAgreement — heavier but runs on weak
/// CPUs). Stored under the `asr_engine` key; the audio worker reads it at go-live.
#[tauri::command]
fn get_asr_engine(db: tauri::State<'_, Db>) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    Ok(setting_get(&conn, "asr_engine").unwrap_or_else(|| "auto".into()))
}

#[tauri::command]
fn set_asr_engine(db: tauri::State<'_, Db>, engine: String) -> Result<(), String> {
    let engine = match engine.as_str() {
        "multilingual" | "english" | "offline" | "auto" => engine,
        _ => "auto".into(),
    };
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    setting_set(&conn, "asr_engine", &engine)
}

#[tauri::command]
fn get_llm_config(db: tauri::State<'_, Db>) -> Result<LlmConfig, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mode = setting_get(&conn, "llm_mode").unwrap_or_else(|| "local".into());
    let default_model = mode2::models::REGISTRY
        .iter()
        .find(|m| m.is_default)
        .map(|m| m.id.to_string())
        .unwrap_or_default();
    let api_provider = setting_get(&conn, "llm_provider").unwrap_or_else(|| "openrouter".into());
    Ok(LlmConfig {
        mode,
        local_model: setting_get(&conn, "local_model").unwrap_or(default_model),
        api_provider: api_provider.clone(),
        api_model: setting_get(&conn, "llm_model"),
        api_key_set: mode2::keyring_has(&api_provider),
        bullet_style: setting_get(&conn, "bullet_style").unwrap_or_else(|| "default".into()),
    })
}

#[tauri::command]
fn set_llm_config(
    db: tauri::State<'_, Db>,
    mode: Option<String>,
    local_model: Option<String>,
    api_provider: Option<String>,
    api_model: Option<String>,
    custom_url: Option<String>,
    bullet_style: Option<String>,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    if let Some(m) = mode {
        setting_set(&conn, "llm_mode", &m)?;
    }
    if let Some(m) = local_model {
        // Only known registry ids: `local_model` is later joined into a file
        // path (models/<id>.gguf), so an unchecked value like `..\..\x` would
        // traverse out of the models dir. Reject anything not in the registry.
        if crate::mode2::models::find(&m).is_none() {
            return Err(format!("unknown model id: {m}"));
        }
        setting_set(&conn, "local_model", &m)?;
    }
    if let Some(p) = api_provider {
        setting_set(&conn, "llm_provider", &p)?;
    }
    if let Some(m) = api_model {
        setting_set(&conn, "llm_model", &m)?;
    }
    if let Some(u) = custom_url {
        setting_set(&conn, "llm_custom_url", &u)?;
    }
    if let Some(s) = bullet_style {
        setting_set(&conn, "bullet_style", &s)?;
    }
    Ok(())
}

// ── Sessions (Phase 6) ──────────────────────────────────────────────
// A session is one call: its own cards, transcript, coverage. The scratch
// session (Phase 3 dev harness) is hidden from this surface.

#[derive(serde::Serialize)]
struct SessionRow {
    id: String,
    title: String,
    kind: String,
    status: String,
    language: String,
    created_at: i64,
    card_count: i64,
}

#[tauri::command]
fn list_sessions(db: tauri::State<'_, Db>) -> Result<Vec<SessionRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.title, s.kind, s.status, s.language, s.created_at,
                    (SELECT COUNT(*) FROM cards c WHERE c.session_id = s.id)
             FROM sessions s
             WHERE s.id <> ?1
             ORDER BY s.created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![live::SCRATCH_SESSION], |r| {
            Ok(SessionRow {
                id: r.get(0)?,
                title: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                language: r.get(4)?,
                created_at: r.get(5)?,
                card_count: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[tauri::command]
fn create_session(
    db: tauri::State<'_, Db>,
    title: String,
    kind: String,
    language: Option<String>,
) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("a session needs a name".into());
    }
    let kind = match kind.as_str() {
        "interview" | "client" | "team" | "investor" | "other" => kind,
        _ => "other".into(),
    };
    // Whitelist to the supported languages (+ auto). Anything else degrades to
    // auto-detect rather than reaching the ASR's per-stream option raw.
    let language = match language.as_deref().map(str::trim) {
        Some("en") => "en",
        Some("es") => "es",
        Some("ru") => "ru",
        Some("uk") => "uk",
        Some("de") => "de",
        _ => "auto",
    };
    let id = uuid::Uuid::new_v4().to_string();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO sessions (id, title, kind, status, language, created_at)
         VALUES (?1, ?2, ?3, 'planned', ?4, strftime('%s','now'))",
        params![id, title, kind, language],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
fn set_session_status(db: tauri::State<'_, Db>, id: String, status: String) -> Result<(), String> {
    let status = match status.as_str() {
        "planned" | "live" | "closed_green" | "closed_red" | "archived" => status,
        _ => return Err("unknown session status".into()),
    };
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE sessions SET status = ?2 WHERE id = ?1",
        params![id, status],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_session(db: tauri::State<'_, Db>, id: String) -> Result<(), String> {
    if id == live::SCRATCH_SESSION {
        return Err("the scratch session cannot be deleted".into());
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    // ON DELETE CASCADE clears the session's cards/transcript/coverage/events.
    conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Serialize)]
struct GenerateReport {
    markdown: String,
    chunks: usize,
    cards: usize,
    warnings: Vec<String>,
    imported: Option<store::ImportReport>,
}

#[derive(serde::Serialize, Clone)]
struct IngestProgress {
    done: usize,
    total: usize,
}

/// Raw material → draft cards via the configured LLM engine. `auto: false`
/// returns the drafts (markdown) for review in the import box; `auto: true`
/// imports them straight into the corpus (owner decision: hybrid).
#[tauri::command]
async fn generate_cards(
    app: tauri::AppHandle,
    text: String,
    auto: bool,
    session_id: Option<String>,
) -> Result<GenerateReport, String> {
    // Snapshot provider + style under a short lock — generation is long and
    // must never hold the DB.
    let (choice, style) = {
        let db = app.state::<Db>();
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let choice = live::resolve_provider(&app, &conn).ok_or(
            "no engine configured — download a local model or add an API key in settings",
        )?;
        let style = setting_get(&conn, "bullet_style").unwrap_or_else(|| "default".into());
        (choice, style)
    };

    let app2 = app.clone();
    let report = ingest::generate_drafts(&choice, &text, &style, move |done, total| {
        app2.emit_to("dashboard", "ingest:progress", IngestProgress { done, total })
            .ok();
    })
    .await?;

    let imported = if auto {
        let db = app.state::<Db>();
        let embedder = app.state::<Arc<Embedder>>();
        Some(import_markdown(
            &db,
            &embedder,
            &report.markdown,
            "en",
            session_id.as_deref(),
        )?)
    } else {
        None
    };
    Ok(GenerateReport {
        markdown: report.markdown,
        chunks: report.chunks,
        cards: report.cards,
        warnings: report.warnings,
        imported,
    })
}

/// Backfills missing short/long bullet variants so the length setting can
/// restyle the whole corpus instantly. No-op when everything has variants.
#[tauri::command]
async fn adapt_corpus(app: tauri::AppHandle) -> Result<usize, String> {
    let (choice, work) = {
        let db = app.state::<Db>();
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let work = store::cards_needing_variants(&conn)?;
        if work.is_empty() {
            return Ok(0);
        }
        let choice = live::resolve_provider(&app, &conn).ok_or(
            "no engine configured — download a local model or add an API key in settings",
        )?;
        (choice, work)
    };
    let app2 = app.clone();
    let app3 = app.clone();
    let adapted = ingest::adapt_variants(
        &choice,
        work,
        move |bullet_id, short, long| {
            let db = app3.state::<Db>();
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            store::set_bullet_variants(&conn, bullet_id, short, long)
        },
        move |done, total| {
            app2.emit_to("dashboard", "adapt:progress", IngestProgress { done, total })
                .ok();
        },
    )
    .await?;
    // Restyle whatever the overlay is currently showing.
    live::restyle_current_card(&app).ok();
    Ok(adapted)
}

/// Re-emits the overlay's current card in the active bullet-length style —
/// called after the style setting changes so the switch is instant.
#[tauri::command]
fn restyle_card(app: tauri::AppHandle) -> Result<(), String> {
    live::restyle_current_card(&app)
}

// ── Appearance (Phase 6): accent, light/dark, overlay transparency ──

#[derive(serde::Serialize, Clone)]
struct Appearance {
    accent: String,
    theme: String,
    /// Overlay card opacity, 40..100 (%). 100 = fully opaque (default).
    overlay_opacity: i64,
}

fn read_appearance(conn: &Connection) -> Appearance {
    Appearance {
        accent: setting_get(conn, "ui_accent").unwrap_or_else(|| "teal".into()),
        theme: setting_get(conn, "ui_theme").unwrap_or_else(|| "dark".into()),
        // Default 90 = a whisper of transparency (owner), still very readable.
        overlay_opacity: setting_get(conn, "overlay_opacity")
            .and_then(|v| v.parse().ok())
            .unwrap_or(90),
    }
}

#[tauri::command]
fn get_appearance(db: tauri::State<'_, Db>) -> Result<Appearance, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    Ok(read_appearance(&conn))
}

#[tauri::command]
fn set_appearance(
    app: tauri::AppHandle,
    db: tauri::State<'_, Db>,
    accent: Option<String>,
    theme: Option<String>,
    overlay_opacity: Option<i64>,
) -> Result<(), String> {
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        if let Some(a) = &accent {
            if matches!(a.as_str(), "coral" | "teal" | "amber") {
                setting_set(&conn, "ui_accent", a)?;
            }
        }
        if let Some(t) = &theme {
            if matches!(t.as_str(), "dark" | "light") {
                setting_set(&conn, "ui_theme", t)?;
            }
        }
        if let Some(o) = overlay_opacity {
            setting_set(&conn, "overlay_opacity", &o.clamp(40, 100).to_string())?;
        }
        let updated = read_appearance(&conn);
        // Overlay lives in its own window — push the change so it restyles live
        // (accent highlight + card opacity) without a reload.
        app.emit_to("overlay", "appearance:changed", &updated).ok();
    }
    Ok(())
}

/// App version (from Cargo) for the corner badge + About.
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Whether the one-time first-run consent screen has been accepted.
#[tauri::command]
fn get_consent(db: tauri::State<'_, Db>) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    Ok(setting_get(&conn, "consent_accepted").as_deref() == Some("1"))
}

/// Record acceptance of the first-run consent + model-download terms (08 §5).
#[tauri::command]
fn accept_consent(db: tauri::State<'_, Db>) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    setting_set(&conn, "consent_accepted", "1")
}

/// Whether the overlay is currently hidden from screen capture (default OFF —
/// no stealth by design; this is a presentation convenience, 00_PRODUCT).
#[tauri::command]
fn get_capture_excluded(db: tauri::State<'_, Db>) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    Ok(setting_get(&conn, "capture_excluded").as_deref() == Some("1"))
}

/// Hide (or reveal) the overlay from screen sharing via the OS capture-exclusion
/// affinity. `on = true` hides it from a Teams/Zoom/OBS share AND from the
/// user's own recordings — the "Show notes" button flips it back to false.
#[tauri::command]
fn set_capture_excluded(
    app: tauri::AppHandle,
    db: tauri::State<'_, Db>,
    on: bool,
) -> Result<(), String> {
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        setting_set(&conn, "capture_excluded", if on { "1" } else { "0" })?;
    }
    if let Some(w) = app.get_webview_window("overlay") {
        w.set_content_protected(on).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Self-healing corpus cleanup: repairs prepared cards whose canonical bullets
/// drifted into prose (older ingested cards, pre-tightener) to the tight
/// Recommended keyword style, re-embedding as it goes. Idempotent; returns the
/// number of cards fixed. Called once on dashboard load.
#[tauri::command]
fn retighten_corpus(
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
) -> Result<usize, String> {
    // Plan under a SHORT read lock, embed OFF the lock (embedding is slow and
    // the live ticker needs db.conn every tick — audit 2026-07-23), then apply
    // under a short write lock. Mirrors import_markdown's discipline.
    let fixes = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        store::retighten_plan(&conn)?
    };
    if fixes.is_empty() {
        return Ok(0);
    }
    let mut vecs = Vec::with_capacity(fixes.len());
    for f in &fixes {
        let bullet_items: Vec<(String, String)> =
            f.keep.iter().map(|(_, t, _, _)| (String::new(), t.clone())).collect();
        let bullet_embs = embedder.embed_passages(&bullet_items)?;
        let joined = f.keep.iter().map(|(_, t, _, _)| t.clone()).collect::<Vec<_>>().join("; ");
        let card_vec = embedder
            .embed_passages(&[(f.title.clone(), joined)])?
            .into_iter()
            .next()
            .unwrap_or_default();
        let bullet_vecs: Vec<(String, Vec<f32>)> = f
            .keep
            .iter()
            .map(|(bid, _, _, _)| bid.clone())
            .zip(bullet_embs)
            .collect();
        let fts_text = f.keep.iter().map(|(_, t, _, _)| t.clone()).collect::<Vec<_>>().join(" ");
        vecs.push(store::RetightenVecs { card_vec, bullet_vecs, fts_text });
    }
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::retighten_apply(&mut conn, &fixes, &vecs)
}

#[tauri::command]
fn set_api_key(provider: String, key: String) -> Result<(), String> {
    if key.trim().is_empty() {
        mode2::keyring_delete(&provider).ok();
        Ok(())
    } else {
        mode2::keyring_set(&provider, key.trim())
    }
}

/// The overlay reports its content height (logical px) so the window can
/// grow for wrapping bullets instead of clipping them.
#[tauri::command]
fn fit_overlay_height(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    const MIN_H: f64 = 120.0;
    const MAX_H: f64 = 520.0;
    let overlay = app
        .get_webview_window("overlay")
        .ok_or("overlay window missing")?;
    let clamped = height.clamp(MIN_H, MAX_H);
    let width = overlay
        .outer_size()
        .ok()
        .map(|s| {
            let scale = overlay.scale_factor().unwrap_or(1.0);
            s.width as f64 / scale
        })
        .unwrap_or(440.0);
    overlay
        .set_size(tauri::LogicalSize::new(width, clamped))
        .map_err(|e| e.to_string())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // sqlite-vec must be registered before any connection opens.
    db::register_vec_extension();

    let embedder = Arc::new(Embedder::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            // Closing the main dashboard window quits the whole app. Otherwise the
            // frameless, always-on-top overlay lives in its own window, so closing
            // the dashboard would leave that widget on screen and keep the process
            // (ticker, audio capture, overlay cursor-poll) alive in the background.
            if window.label() == "dashboard" {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    window.app_handle().exit(0);
                }
            }
        })
        .manage(overlay_input::Zones::default())
        .manage(live::LiveState::default())
        .manage(audio::AudioState::default())
        .manage(std::sync::Arc::new(mode2::local::LocalEngine::default()))
        .manage(embedder.clone())
        .invoke_handler(tauri::generate_handler![
            boot_info,
            import_cards,
            import_folder,
            list_cards,
            delete_card,
            wipe_corpus,
            query_cards,
            fit_overlay_height,
            overlay_input::set_interactive_zones,
            live::feed_transcript,
            live::reset_live,
            live::set_thresholds,
            live::get_thresholds,
            live::panic_now,
            live::set_active_session,
            live::clear_active_session,
            live::get_active_session,
            live::close_session,
            live::session_report,
            live::reopen_session,
            audio::audio_status,
            audio::start_audio,
            audio::stop_audio,
            list_models,
            download_model,
            delete_model,
            get_llm_config,
            set_llm_config,
            get_asr_engine,
            set_asr_engine,
            generate_cards,
            adapt_corpus,
            restyle_card,
            retighten_corpus,
            list_sessions,
            create_session,
            set_session_status,
            delete_session,
            list_session_cards,
            add_library_cards_to_session,
            promote_cards_to_library,
            session_transcript,
            preflight::preflight_research,
            get_appearance,
            set_appearance,
            app_version,
            get_consent,
            accept_consent,
            get_capture_excluded,
            set_capture_excluded,
            set_api_key
        ])
        .setup(move |app| {
            // Portable: everything in one folder next to the app. Migrate any
            // existing DB/models/cache out of the old per-user AppData location
            // so upgrading users keep their data.
            let old_app_data = app.path().app_data_dir().ok();
            let cwd = std::env::current_dir().unwrap_or_default();
            paths::migrate_if_needed(
                old_app_data.as_deref(),
                &[cwd.join(".fastembed_cache"), cwd.join("src-tauri/.fastembed_cache")],
            );
            let data_dir = paths::data_dir();
            let db_path = data_dir.join("anchor.db");
            let conn = db::open_and_migrate(&db_path)?;
            let (_, dims) =
                db::ensure_embedding_config(&conn, embed::MODEL_ID, embed::DIMS as i64)?;
            db::ensure_vec_tables(&conn, dims)?;
            // Refuse to run with a mismatched embedding space — silent mixing
            // corrupts retrieval invisibly.
            db::check_embedding_compat(&conn, embed::MODEL_ID, embed::DIMS as i64)
                .map_err(std::io::Error::other)?;
            live::ensure_scratch_session(&conn)?;
            let capture_excluded = setting_get(&conn, "capture_excluded").as_deref() == Some("1");
            tracing::info!(path = %db_path.display(), dims, "database ready");
            app.manage(Db {
                conn: Mutex::new(conn),
                path: db_path,
            });
            live::spawn_ticker(app.handle().clone());

            // Pre-warm the embedding model off-thread (first run downloads it).
            let warm = embedder.clone();
            std::thread::spawn(move || {
                if let Err(e) = warm.embed_query("warmup") {
                    tracing::warn!(error = %e, "embedder pre-warm failed");
                }
            });

            let overlay = app
                .get_webview_window("overlay")
                .expect("overlay window declared in tauri.conf.json");
            // Re-apply the saved screen-share preference (default OFF — no stealth).
            if capture_excluded {
                let _ = overlay.set_content_protected(true);
            }
            overlay_input::spawn_poll_loop(overlay);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Anchor");
}
