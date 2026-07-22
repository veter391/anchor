// Anchor — live notes overlay. Prepared, not prompted.
// Phase 1: shell, two windows, SQLite schema on boot, hardcoded card.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod db;
mod overlay_input;

use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

struct Db {
    conn: Mutex<Connection>,
    path: PathBuf,
}

#[derive(serde::Serialize)]
struct BootInfo {
    db_path: String,
    schema_version: i64,
    tables: Vec<String>,
}

#[tauri::command]
fn boot_info(db: tauri::State<'_, Db>) -> Result<BootInfo, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let tables = db::table_names(&conn).map_err(|e| e.to_string())?;
    Ok(BootInfo {
        db_path: db.path.display().to_string(),
        schema_version: db::SCHEMA_VERSION,
        tables,
    })
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .manage(overlay_input::Zones::default())
        .invoke_handler(tauri::generate_handler![
            boot_info,
            overlay_input::set_interactive_zones
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("anchor.db");
            let conn = db::open_and_migrate(&db_path)?;
            tracing::info!(path = %db_path.display(), "database ready");
            app.manage(Db {
                conn: Mutex::new(conn),
                path: db_path,
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
