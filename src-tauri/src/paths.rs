//! Portable data location (owner decision 2026-07-23): the database, the
//! downloaded models and the embedding cache all live in ONE folder next to
//! the executable, so Anchor is self-contained — one delete removes
//! everything, nothing is scattered across the machine. Ships as an
//! extract-and-run folder (a read-only install dir would need the AppData
//! fallback; that lands at Phase 8).

use std::path::{Path, PathBuf};

/// The one data folder, next to the executable.
pub fn data_dir() -> PathBuf {
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("data");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Embedding-model cache (fastembed downloads EmbeddingGemma here).
pub fn cache_dir() -> PathBuf {
    let dir = data_dir().join("model-cache");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Recursively copy a directory's contents (best-effort; skips failures).
fn copy_dir(from: &Path, to: &Path) {
    if std::fs::create_dir_all(to).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for e in entries.flatten() {
        let src = e.path();
        let dst = to.join(e.file_name());
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

/// One-time move of data from the OLD per-user locations into the portable
/// folder, so upgrading users keep their cards and downloaded models. Runs
/// only when the portable DB does not yet exist. `old_app_data` is Tauri's
/// resolved app-data dir; `old_cache` candidates are legacy fastembed caches.
pub fn migrate_if_needed(old_app_data: Option<&Path>, old_cache_candidates: &[PathBuf]) {
    let data = data_dir();
    let db = data.join("anchor.db");
    if db.exists() {
        return; // already portable
    }

    // DB (+ its WAL/SHM sidecars) and the models folder from AppData.
    if let Some(old) = old_app_data {
        let old_db = old.join("anchor.db");
        if old_db.exists() {
            let _ = std::fs::copy(&old_db, &db);
            for sfx in ["anchor.db-wal", "anchor.db-shm"] {
                let s = old.join(sfx);
                if s.exists() {
                    let _ = std::fs::copy(&s, data.join(sfx));
                }
            }
            let old_models = old.join("models");
            if old_models.is_dir() {
                copy_dir(&old_models, &data.join("models"));
            }
            tracing::info!("migrated DB + models from AppData into the portable folder");
        }
    }

    // Embedding cache from wherever fastembed put it, so it isn't re-downloaded.
    let cache = cache_dir();
    if std::fs::read_dir(&cache).map(|mut d| d.next().is_none()).unwrap_or(true) {
        for cand in old_cache_candidates {
            if cand.is_dir() {
                copy_dir(cand, &cache);
                tracing::info!(from = %cand.display(), "migrated embedding cache");
                break;
            }
        }
    }
}
