// Anchor — live notes overlay. Prepared, not prompted.
// Phase 2: corpus import, local embeddings, hybrid retrieval.

pub mod cards;
pub mod db;
pub mod embed;
pub mod overlay_input;
pub mod search;
pub mod store;

use embed::Embedder;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

struct Db {
    conn: Mutex<Connection>,
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
fn boot_info(db: tauri::State<'_, Db>, embedder: tauri::State<'_, Arc<Embedder>>) -> Result<BootInfo, String> {
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

#[tauri::command]
fn import_cards(
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
    markdown: String,
    default_lang: Option<String>,
) -> Result<store::ImportReport, String> {
    let parsed = cards::parse_markdown(&markdown, default_lang.as_deref().unwrap_or("en"));
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::import_cards(&mut conn, &embedder, parsed, None)
}

#[tauri::command]
fn list_cards(db: tauri::State<'_, Db>) -> Result<Vec<store::CardRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::list_cards(&conn)
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
/// pushed to the overlay window.
#[tauri::command]
fn query_cards(
    app: tauri::AppHandle,
    db: tauri::State<'_, Db>,
    embedder: tauri::State<'_, Arc<Embedder>>,
    text: String,
) -> Result<QueryResult, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let matches = search::query_cards(&conn, &embedder, &text)?;
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
        .manage(overlay_input::Zones::default())
        .manage(embedder.clone())
        .invoke_handler(tauri::generate_handler![
            boot_info,
            import_cards,
            list_cards,
            wipe_corpus,
            query_cards,
            overlay_input::set_interactive_zones
        ])
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("anchor.db");
            let conn = db::open_and_migrate(&db_path)?;
            let (_, dims) = db::ensure_embedding_config(&conn, embed::MODEL_ID, embed::DIMS as i64)?;
            db::ensure_vec_tables(&conn, dims)?;
            tracing::info!(path = %db_path.display(), dims, "database ready");
            app.manage(Db {
                conn: Mutex::new(conn),
                path: db_path,
            });

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
