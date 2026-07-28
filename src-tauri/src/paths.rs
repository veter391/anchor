//! Data location. The database, downloaded models and embedding cache live in
//! ONE folder. The PORTABLE build keeps it as `data/` next to the executable
//! (owner decision 2026-07-23) — self-contained, one delete removes everything.
//! The INSTALLED build keeps it in a per-user folder outside the (churned)
//! install dir so upgrades never wipe user data — see `data_dir`.
//!
//! The portable folder holds the transcript DB and downloaded models, so on
//! Windows its ACL is restricted to the current user + SYSTEM + Administrators
//! (see `harden_data_dir_acl`) — otherwise, extracted to a shared location, it
//! would inherit a world-readable ACL and expose the transcripts to other
//! accounts on the machine.

use std::path::{Path, PathBuf};

/// A stable per-user data folder (installed build / current_exe fallback).
fn per_user_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("AnchorData")
}

/// The one data folder holding the DB, downloaded models and cache.
///
/// - **Portable build:** `data/` next to the executable — self-contained, one
///   delete removes everything (owner decision 2026-07-23).
/// - **Installed build (NSIS):** a per-user folder OUTSIDE the install dir. The
///   installer drops a `.installed` marker next to the exe; the install dir
///   (`%LOCALAPPDATA%\Anchor`) is churned on every upgrade/uninstall, so the
///   DB, transcripts and ~1 GB of models must not live under it or an upgrade
///   would wipe them. `AnchorData` is a sibling the uninstaller never touches.
pub fn data_dir() -> PathBuf {
    let dir = match std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        Some(parent) if parent.join(".installed").exists() => per_user_data_dir(),
        Some(parent) => parent.join("data"),
        None => {
            tracing::warn!("current_exe() failed; using a stable per-user data dir");
            per_user_data_dir()
        }
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Embedding-model cache (fastembed downloads EmbeddingGemma here).
pub fn cache_dir() -> PathBuf {
    let dir = data_dir().join("model-cache");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Restrict the portable data folder to the current user (Windows). The folder
/// carries the transcript DB and downloaded models; extracted to a shared
/// location it would inherit a world-readable ACL. Runs ONCE (a marker guards
/// it), best-effort — a failure logs and leaves the folder untouched, never
/// blocks startup. Uses `icacls` (the canonical Windows ACL tool; the native
/// `SetNamedSecurityInfo` path is ~80 lines of unsafe for a one-shot op).
///
/// Order matters: the current user often has access ONLY through the inherited
/// `Users`/`Authenticated Users` groups, so we grant the user an EXPLICIT full
/// ACE *before* removing inheritance — otherwise stripping the broad groups
/// would lock the user out of their own data (verified: it does). SYSTEM +
/// Administrators are kept so backup/admin recovery still works, and the
/// inheritable (OI)(CI) flags mean later-downloaded models inherit the same
/// restriction. Once hardened, nothing broadly-readable remains.
#[cfg(windows)]
pub fn harden_data_dir_acl() {
    harden_acl(&data_dir());
}

#[cfg(windows)]
fn harden_acl(dir: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let marker = dir.join(".acl-hardened");
    if marker.exists() {
        return;
    }
    // Without a resolvable current-user name we cannot grant the explicit ACE,
    // so we skip rather than risk a lock-out.
    let user = match std::env::var("USERNAME") {
        Ok(u) if !u.trim().is_empty() => u,
        _ => {
            tracing::warn!("USERNAME unset; skipping data-folder ACL hardening");
            return;
        }
    };
    let path = dir.to_string_lossy().to_string();
    // Directory only — deliberately NOT `/t`. The `(OI)(CI)` flags make these
    // ACEs INHERITABLE, so files created afterwards get an effective ACE. But
    // applying `(OI)(CI)` to an already-existing FILE via `/t` yields an
    // inherit-only ACE that grants that file nothing — it bricks it (found the
    // hard way: it left the freshly-migrated anchor.db with an empty DACL, so
    // SQLite could no longer open it). We run BEFORE migration on an empty
    // folder (see the setup hook), so everything created/copied after simply
    // inherits — no existing file is ever touched.
    let result = std::process::Command::new("icacls")
        .args([
            path.as_str(),
            "/inheritance:r", // drop inherited (world-readable) ACEs
            "/grant:r",
            &format!("{user}:(OI)(CI)F"), // current user — GRANTED FIRST
            "*S-1-5-18:(OI)(CI)F",        // SYSTEM
            "*S-1-5-32-544:(OI)(CI)F",    // BUILTIN\Administrators
            "/q",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match result {
        Ok(o) if o.status.success() => {
            let _ = std::fs::write(&marker, b"1");
            tracing::info!("restricted the data folder ACL to the current user");
        }
        Ok(o) => tracing::warn!(
            code = ?o.status.code(),
            "icacls hardening did not succeed; leaving folder permissions unchanged"
        ),
        Err(e) => tracing::warn!(error = %e, "could not run icacls to harden the data folder"),
    }
}

/// No-op off Windows (portable-folder ACL hardening is Windows-specific).
#[cfg(not(windows))]
pub fn harden_data_dir_acl() {}

/// Recursively copy a directory's contents. Does NOT follow symlinks/junctions
/// (audit 2026-07-23: `is_dir()` follows them, letting a planted link redirect
/// the copy outside the source tree). Best-effort; skips failures.
/// Returns `true` only if every file under `from` was copied. Callers that must
/// not mark a migration "done" on a torn copy (models — #6) gate on this.
fn copy_dir(from: &Path, to: &Path) -> bool {
    if std::fs::create_dir_all(to).is_err() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(from) else {
        return false;
    };
    let mut ok = true;
    for e in entries.flatten() {
        let src = e.path();
        // symlink_metadata does not traverse links.
        let Ok(md) = std::fs::symlink_metadata(&src) else {
            ok = false;
            continue;
        };
        if md.file_type().is_symlink() {
            continue; // never copy through a link (intentional skip, not a failure)
        }
        let dst = to.join(e.file_name());
        if md.is_dir() {
            ok &= copy_dir(&src, &dst);
        } else if std::fs::copy(&src, &dst).is_err() {
            ok = false;
        }
    }
    ok
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
            // Only mark done on a COMPLETE copy — otherwise a torn copy (disk
            // full / locked file) would be skipped forever, leaving a partial
            // model set. On failure, leave no marker so the next launch retries.
            if copy_dir(&old_models, &data.join("models")) {
                let _ = std::fs::write(&marker, b"1");
                tracing::info!("migrated models into the portable folder");
            } else {
                tracing::warn!("model migration incomplete; will retry next launch");
            }
        });
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn hardening_keeps_files_accessible_to_the_owner() {
        // Two properties, both load-bearing:
        //  1. A file that ALREADY exists when hardening runs must stay openable
        //     read+write. (Regression guard: applying inheritance flags with
        //     `/t` gave existing files an inherit-only, access-nothing DACL and
        //     bricked the migrated anchor.db — SQLite could not open it.)
        //  2. A file created AFTER hardening must inherit access.
        // Project-local temp dir (OWNER-RULES §9: not the OS temp).
        use std::fs::OpenOptions;
        let dir = PathBuf::from("target").join(format!("acl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("existing.bin");
        std::fs::write(&existing, b"db-like").unwrap();

        harden_acl(&dir);

        // (1) The pre-existing file opens read+write, exactly as SQLite opens
        // the DB. This is what actually broke before the /t was removed.
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&existing)
            .expect("a file present before hardening must stay openable (not bricked)");
        // (2) A new file inherits access.
        std::fs::write(dir.join("after.bin"), b"y")
            .expect("owner must still be able to create files after hardening");
        assert!(dir.join(".acl-hardened").exists(), "success marker written");

        // Reset the ACL so the dir can be cleaned up, then remove it.
        let _ = std::process::Command::new("icacls")
            .args([dir.to_string_lossy().as_ref(), "/reset", "/t", "/c", "/q"])
            .output();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
