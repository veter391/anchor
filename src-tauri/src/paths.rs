//! Portable data location (owner decision 2026-07-23): the database, the
//! downloaded models and the embedding cache all live in ONE folder ('data/')
//! next to the executable, so Anchor is self-contained — one delete removes
//! everything, nothing is scattered across the machine. Ships as an
//! extract-and-run folder.
//!
//! Note for Phase 8 ship: harden the folder ACL to the current user (a
//! portable folder in a shared location would otherwise be world-readable) —
//! tracked, not done here (single-user dev today). See 10_RESEARCH_LOG.

use std::path::{Path, PathBuf};

/// The one data folder, next to the executable.
pub fn data_dir() -> PathBuf {
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| {
            // current_exe() failing is rare; fall back to a STABLE per-user
            // location, never the unpredictable process CWD (audit 2026-07-23).
            tracing::warn!("current_exe() failed; using a stable per-user data dir");
            std::env::var_os("LOCALAPPDATA")
                .or_else(|| std::env::var_os("APPDATA"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("Anchor")
        });
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

/// Recursively copy a directory's contents. Does NOT follow symlinks/junctions
/// (audit 2026-07-23: `is_dir()` follows them, letting a planted link redirect
/// the copy outside the source tree). Best-effort; skips failures.
fn copy_dir(from: &Path, to: &Path) {
    if std::fs::create_dir_all(to).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for e in entries.flatten() {
        let src = e.path();
        // symlink_metadata does not traverse links.
        let Ok(md) = std::fs::symlink_metadata(&src) else {
            continue;
        };
        if md.file_type().is_symlink() {
            continue; // never copy through a link
        }
        let dst = to.join(e.file_name());
        if md.is_dir() {
            copy_dir(&src, &dst);
        } else {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

/// Folds a WAL into the main DB file so a single-file copy is consistent.
/// (Audit 2026-07-23: copying `-wal`/`-shm` alongside `-db` can drop
/// committed-but-uncheckpointed changes or tear pages.)
fn checkpoint(db: &Path) {
    if let Ok(conn) = rusqlite::Connection::open(db) {
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| {
            Ok::<_, rusqlite::Error>(())
        });
    }
}

/// One-time move of data from the OLD per-user locations into the portable
/// folder, so upgrading users keep their cards and downloaded models. Runs
/// only when the portable data is not yet present.
pub fn migrate_if_needed(old_app_data: Option<&Path>, old_cache_candidates: &[PathBuf]) {
    let Some(old) = old_app_data else { return };
    let old_db = old.join("anchor.db");
    if !old_db.exists() {
        return; // nothing to migrate
    }
    let data = data_dir();

    // DB — synchronous, checkpointed, ATOMIC. Small (a few MB), safe on the
    // main thread. Copy into a temp file then rename, so the final `anchor.db`
    // (the guard below) appears only after a complete copy — a killed migration
    // never leaves a torn DB that the next launch treats as done.
    let db = data.join("anchor.db");
    if !db.exists() {
        checkpoint(&old_db);
        let tmp = data.join(".anchor.db.migrating");
        let _ = std::fs::remove_file(&tmp);
        if std::fs::copy(&old_db, &tmp).is_ok() && std::fs::rename(&tmp, &db).is_ok() {
            tracing::info!("migrated DB into the portable folder");
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    // Embedding cache — synchronous, but small-ish (~300 MB) and it MUST land
    // before the embedder pre-warm downloads a fresh copy into the same dir
    // (that would race). Only if the cache is still empty.
    let cache = cache_dir();
    let cache_empty = std::fs::read_dir(&cache)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    if cache_empty {
        for cand in old_cache_candidates {
            if cand.is_dir() {
                copy_dir(cand, &cache);
                tracing::info!(from = %cand.display(), "migrated embedding cache");
                break;
            }
        }
    }

    // Models — the big one (~1 GB). Not needed until a Mode-2 call fires, so
    // copy OFF the main thread (no startup freeze). A marker guards it so a
    // torn copy retries next launch instead of being skipped forever.
    let marker = data.join(".models-migrated");
    let old_models = old.join("models");
    if !marker.exists() && old_models.is_dir() {
        std::thread::spawn(move || {
            copy_dir(&old_models, &data.join("models"));
            let _ = std::fs::write(&marker, b"1");
            tracing::info!("migrated models into the portable folder");
        });
    }
}
