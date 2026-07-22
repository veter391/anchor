// Anchor — live notes overlay. Prepared, not prompted.
// Phase 2 (+review fixes): corpus import, local embeddings, hybrid retrieval.

pub mod cards;
pub mod db;
pub mod embed;
pub mod live;
pub mod matcher;
pub mod overlay_input;
pub mod search;
pub mod store;

use embed::Embedder;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

pub(crate) struct Db {
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
) -> Result<store::ImportReport, String> {
    let parsed = cards::parse_markdown(markdown, default_lang);
    let vectors = store::embed_import(embedder, &parsed)?;
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::write_import(&mut conn, parsed, vectors, None)
}

#[tauri::command]
fn import_cards(
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
    markdown: String,
    default_lang: Option<String>,
) -> Result<store::ImportReport, String> {
    import_markdown(
        &db,
        &embedder,
        &markdown,
        default_lang.as_deref().unwrap_or("en"),
    )
}

/// Recursively reads every .md file under `path` and imports the lot.
#[tauri::command]
fn import_folder(
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
    path: String,
    default_lang: Option<String>,
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

    let embedder = Arc::new(Embedder::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(overlay_input::Zones::default())
        .manage(live::LiveState::default())
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
            live::get_thresholds
        ])
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
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
            overlay_input::spawn_poll_loop(overlay);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Anchor");
}
