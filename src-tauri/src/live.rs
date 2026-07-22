//! The live session runtime: transcript feed → rolling windows → ticker →
//! match engine → events to the overlay and the debug panel.
//!
//! THEIR speech drives card selection; YOUR speech drives bullet coverage
//! (02_ARCHITECTURE §5). Every tick: embed outside the DB lock, then one
//! short lock for search + logging. Jumps are logged to card_events with
//! the runner-up — that log is the threshold-tuning dataset.

use crate::matcher::{Candidate, Decision, MatchEngine, RollingWindow, Thresholds};
use crate::{search, store};
use rusqlite::params;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

/// Until Phase 6 brings real sessions, live runs log under this one.
pub const SCRATCH_SESSION: &str = "scratch";

pub struct LiveState {
    windows: Mutex<Windows>,
    pub engine: Mutex<MatchEngine>,
    dirty: AtomicBool,
    /// Mode-2 debounce: true while an assembly is in flight, plus the
    /// question text we last assembled for (so we don't re-fire every tick
    /// while the same unmatched question sits in the window).
    mode2_inflight: AtomicBool,
    mode2_last_q: Mutex<String>,
}

struct Windows {
    them: RollingWindow,
    me: RollingWindow,
    origin: Instant,
}

impl Default for LiveState {
    fn default() -> Self {
        Self {
            windows: Mutex::new(Windows {
                them: RollingWindow::default(),
                me: RollingWindow::default(),
                origin: Instant::now(),
            }),
            engine: Mutex::new(MatchEngine::new(Thresholds::default())),
            dirty: AtomicBool::new(false),
            mode2_inflight: AtomicBool::new(false),
            mode2_last_q: Mutex::new(String::new()),
        }
    }
}

fn lock_or_recover<'a, T>(m: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ── Commands ────────────────────────────────────────────────────────

#[tauri::command]
pub fn feed_transcript(
    live: tauri::State<'_, LiveState>,
    speaker: String,
    text: String,
) -> Result<(), String> {
    feed_transcript_internal(&live, &speaker, &text)
}

/// Shared entry point for both the manual command and the audio worker.
pub fn feed_transcript_internal(
    live: &LiveState,
    speaker: &str,
    text: &str,
) -> Result<(), String> {
    let mut w = lock_or_recover(&live.windows);
    let ts_ms = w.origin.elapsed().as_millis() as u64;
    match speaker {
        "them" => w.them.push(ts_ms, text.to_string()),
        "me" => w.me.push(ts_ms, text.to_string()),
        other => return Err(format!("unknown speaker: {other}")),
    }
    live.dirty.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn reset_live(
    live: tauri::State<'_, LiveState>,
    db: tauri::State<'_, crate::Db>,
) -> Result<(), String> {
    {
        let mut w = lock_or_recover(&live.windows);
        w.them.clear();
        w.me.clear();
        w.origin = Instant::now();
    }
    lock_or_recover(&live.engine).reset();
    live.mode2_inflight.store(false, Ordering::SeqCst);
    lock_or_recover(&live.mode2_last_q).clear();
    // Clear the scratch session's live rows so a re-run doesn't accumulate
    // duplicate coverage/event rows (coverage is sticky in memory; the DB is
    // the record, and the engine reset just wiped the in-memory flags).
    let conn = db.conn.lock().unwrap_or_else(|p| p.into_inner());
    conn.execute(
        "DELETE FROM coverage WHERE session_id = ?1",
        rusqlite::params![SCRATCH_SESSION],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM card_events WHERE session_id = ?1",
        rusqlite::params![SCRATCH_SESSION],
    )
    .map_err(|e| e.to_string())?;
    live.dirty.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn set_thresholds(live: tauri::State<'_, LiveState>, thresholds: Thresholds) {
    lock_or_recover(&live.engine).thresholds = thresholds;
}

#[tauri::command]
pub fn get_thresholds(live: tauri::State<'_, LiveState>) -> Thresholds {
    lock_or_recover(&live.engine).thresholds
}

/// Panic hotkey / button: show the three universal anchors instantly.
#[tauri::command]
pub fn panic_now(app: tauri::AppHandle) {
    let card = crate::mode2::panic_card();
    app.emit_to("overlay", "card:assembled", &card).ok();
}

// ── Events ──────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct DebugCandidate {
    card_id: String,
    title: String,
    score: f64,
    vec_rank: Option<usize>,
    bm25_rank: Option<usize>,
    vec_cos: Option<f64>,
}

#[derive(Serialize, Clone)]
struct DebugState {
    them_text: String,
    me_text: String,
    top: Vec<DebugCandidate>,
    thresholds: Thresholds,
    current_card: Option<String>,
    challenger: Option<(String, u32)>,
    no_confidence: bool,
    bullet_sims: Vec<f64>,
    tick_ms: u128,
}

#[derive(Serialize, Clone)]
struct CoverageUpdate {
    card_id: String,
    covered: Vec<bool>,
}

// ── The ticker ──────────────────────────────────────────────────────

const TICK: Duration = Duration::from_millis(300);

pub fn spawn_ticker(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let live = app.state::<LiveState>();
        // Tick on new content OR while a challenger streak is pending —
        // hysteresis needs consecutive evaluations, and the decisive ones
        // often fall in the silence right after a question.
        let dirty = live.dirty.swap(false, Ordering::SeqCst);
        let pending = lock_or_recover(&live.engine).has_pending_challenger();
        if !dirty && !pending {
            continue;
        }
        let started = Instant::now();
        if let Err(e) = tick(&app) {
            tracing::warn!(error = %e, "live tick failed");
        }
        tracing::trace!(elapsed_ms = started.elapsed().as_millis() as u64, "tick");
    });
}

fn tick(app: &tauri::AppHandle) -> Result<(), String> {
    let live = app.state::<LiveState>();
    let embedder = app.state::<std::sync::Arc<crate::embed::Embedder>>();
    let db = app.state::<crate::Db>();

    let (them_text, me_text) = {
        let w = lock_or_recover(&live.windows);
        (w.them.text(), w.me.text())
    };

    // Slow model work FIRST, no locks held.
    let them_vec = if them_text.is_empty() {
        None
    } else {
        Some(search::embed_query_text(&embedder, &them_text)?)
    };
    let me_vec = if me_text.is_empty() {
        None
    } else {
        Some(search::embed_query_text(&embedder, &me_text)?)
    };

    let tick_start = Instant::now();
    // Poison-tolerant, like the windows/engine locks: a one-off panic elsewhere
    // must not permanently kill live matching.
    let conn = db.conn.lock().unwrap_or_else(|p| p.into_inner());

    // ── Level 1: card selection from THEIR window ──
    let mut top: Vec<DebugCandidate> = Vec::new();
    let mut no_confidence = false;
    let mut jumped: Option<store::CardRow> = None;

    if let Some(qvec) = &them_vec {
        let matches = search::query_cards_with_vec(&conn, qvec, &them_text)?;
        // Score for the hysteresis thresholds is the REAL cosine similarity,
        // not the RRF rank score: RRF ranks are near-identical across the top
        // few cards (rank-1 in both legs ≈ 1.0 for everything), so a rank-based
        // margin can never separate the current card from a challenger. Cosine
        // is the actual "how close is this topic" signal the thresholds want.
        // A keyword-only hit (no vector rank) gets a moderate floor — the exact
        // term matched, which is a real signal for names/numbers.
        const BM25_ONLY_SCORE: f64 = 0.6;
        let mut candidates: Vec<Candidate> = matches
            .iter()
            .map(|m| Candidate {
                card_id: m.card_id.clone(),
                score: m
                    .vec_distance
                    .map(|d| {
                        // cos = 1 − L2²/2 holds for unit vectors when d is the
                        // (non-squared) Euclidean L2 — sqlite-vec's vec0 default.
                        // Guard in debug builds against a future metric change:
                        // L2 of two unit vectors is in [0, 2].
                        debug_assert!(
                            (0.0..=2.001).contains(&d),
                            "unexpected vec distance {d}; is the vec0 metric still L2?"
                        );
                        1.0 - (d * d) / 2.0
                    })
                    .unwrap_or(BM25_ONLY_SCORE),
            })
            .collect();
        // Order by real similarity so decide()'s `first()` is the closest card.
        candidates.sort_by(|a, b| b.score.total_cmp(&a.score));

        let decision = {
            let mut engine = lock_or_recover(&live.engine);
            engine.decide(&candidates, Instant::now())
        };

        for c in candidates.iter().take(3) {
            let m = matches.iter().find(|m| m.card_id == c.card_id);
            let title = store::get_card(&conn, &c.card_id)?
                .map(|c| c.title)
                .unwrap_or_else(|| c.card_id.clone());
            top.push(DebugCandidate {
                card_id: c.card_id.clone(),
                title,
                score: c.score,
                vec_rank: m.and_then(|m| m.vec_rank),
                bm25_rank: m.and_then(|m| m.bm25_rank),
                // Unit vectors: cosine = 1 − L2²/2.
                vec_cos: m.and_then(|m| m.vec_distance).map(|d| 1.0 - (d * d) / 2.0),
            });
        }

        match decision {
            Decision::Jump { card_id } => {
                if let Some(card) = store::get_card(&conn, &card_id)? {
                    let (runner_id, runner_score) = candidates
                        .iter()
                        .find(|c| c.card_id != card_id)
                        .map(|c| (Some(c.card_id.clone()), Some(c.score)))
                        .unwrap_or((None, None));
                    let ts_ms = lock_or_recover(&live.windows).origin.elapsed().as_millis() as i64;
                    conn.execute(
                        "INSERT INTO card_events
                           (session_id, card_id, ts_ms, mode, score, runner_up, runner_score, fused_rank)
                         VALUES (?1, ?2, ?3, 'retrieved', ?4, ?5, ?6, ?7)",
                        params![
                            SCRATCH_SESSION,
                            card.id,
                            ts_ms,
                            candidates.first().map(|c| c.score),
                            runner_id,
                            runner_score,
                            serde_json::to_string(&top).unwrap_or_default(),
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                    jumped = Some(card);
                }
            }
            Decision::NoConfidence => {
                no_confidence = true;
                maybe_assemble(app, &live, &conn, &them_text);
            }
            Decision::Stay => {}
        }
    }

    // ── Level 2: bullet coverage from MY window on the active card ──
    let mut bullet_sims: Vec<f64> = Vec::new();
    let mut coverage_update: Option<CoverageUpdate> = None;
    let active_card = {
        let engine = lock_or_recover(&live.engine);
        engine.current_card().map(String::from)
    };
    if let (Some(card_id), Some(mvec)) = (&active_card, &me_vec) {
        let bullets = store::bullet_vectors_for_card(&conn, card_id)?;
        if !bullets.is_empty() {
            bullet_sims = bullets
                .iter()
                .map(|(_, bv)| dot(mvec, bv) as f64)
                .collect();
            let newly = {
                let mut engine = lock_or_recover(&live.engine);
                engine.update_coverage(card_id, &bullet_sims)
            };
            if let Some(covered) = newly {
                let ts_ms = lock_or_recover(&live.windows).origin.elapsed().as_millis() as i64;
                for ((bullet_id, _), flag) in bullets.iter().zip(&covered) {
                    if *flag {
                        conn.execute(
                            "INSERT INTO coverage (session_id, card_id, bullet_id, covered, score, ts_ms)
                             VALUES (?1, ?2, ?3, 1, NULL, ?4)",
                            params![SCRATCH_SESSION, card_id, bullet_id, ts_ms],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
                coverage_update = Some(CoverageUpdate {
                    card_id: card_id.clone(),
                    covered,
                });
            }
        }
    }

    let debug = {
        let engine = lock_or_recover(&live.engine);
        DebugState {
            them_text,
            me_text,
            top,
            thresholds: engine.thresholds,
            current_card: engine.current_card().map(String::from),
            challenger: engine
                .challenger_streak()
                .map(|(id, n)| (id.to_string(), n)),
            no_confidence,
            bullet_sims,
            tick_ms: tick_start.elapsed().as_millis(),
        }
    };
    drop(conn);

    if let Some(card) = jumped {
        app.emit_to("overlay", "card:show", &card)
            .map_err(|e| e.to_string())?;
    }
    if let Some(cu) = coverage_update {
        app.emit_to("overlay", "coverage:update", &cu)
            .map_err(|e| e.to_string())?;
    }
    app.emit_to("dashboard", "match:debug", &debug)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ── Mode 2: assembly on an unmatched question ───────────────────────

/// Fires Mode-2 assembly when the match engine has no confident card. Shows
/// the panic card instantly (never a spinner in the overlay — 06_DESIGN),
/// then assembles in the background and swaps the assembled card in. Debounced
/// so the same lingering question does not re-trigger every tick.
fn maybe_assemble(
    app: &tauri::AppHandle,
    live: &LiveState,
    conn: &rusqlite::Connection,
    question: &str,
) {
    let q = question.trim();
    if q.split_whitespace().count() < 3 {
        return; // too little to assemble from
    }
    // Debounce: skip if one is already running, or we already assembled for a
    // near-identical question.
    if live.mode2_inflight.load(Ordering::SeqCst) {
        return;
    }
    {
        let last = lock_or_recover(&live.mode2_last_q);
        if crate::audio::text_overlap(q, &last) >= 0.6 {
            return;
        }
    }

    let choice = match resolve_provider(app, conn) {
        Some(c) => c,
        None => return, // no provider configured — stay on the panic card
    };
    let style = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'bullet_style'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "default".to_string());

    // Gather the user's material (all prepared bullets) while we hold the lock.
    let material = match gather_material(conn) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "mode2: gather material failed");
            return;
        }
    };

    live.mode2_inflight.store(true, Ordering::SeqCst);
    *lock_or_recover(&live.mode2_last_q) = q.to_string();

    // Panic card instantly, as the filler while assembly runs.
    let panic = crate::mode2::panic_card();
    app.emit_to("overlay", "card:assembled", &panic).ok();

    let app2 = app.clone();
    let embedder = app.state::<std::sync::Arc<crate::embed::Embedder>>().inner().clone();
    let question = q.to_string();
    // Off the ticker thread: network + embedding, then emit the result.
    tauri::async_runtime::spawn(async move {
        let result =
            crate::mode2::assemble(&choice, &embedder, &material, &question, &style).await;
        let live = app2.state::<LiveState>();
        match result {
            Ok(card) => {
                app2.emit_to("overlay", "card:assembled", &card).ok();
                app2.emit_to("dashboard", "mode2:done", &card).ok();
            }
            Err(e) => {
                tracing::warn!(error = %e, "mode2 assembly failed");
                app2.emit_to("dashboard", "mode2:error", &e).ok();
            }
        }
        live.mode2_inflight.store(false, Ordering::SeqCst);
    });
}

/// Provider from settings. `llm_mode` = "local" (default, free) or "api".
/// Local uses the active downloaded model; API pulls its key from the OS
/// keyring (never our DB — OWNER-RULES).
pub(crate) fn resolve_provider(
    app: &tauri::AppHandle,
    conn: &rusqlite::Connection,
) -> Option<crate::mode2::ProviderChoice> {
    let get = |key: &str| -> Option<String> {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };
    let mode = get("llm_mode").unwrap_or_else(|| "local".to_string());
    if mode == "api" {
        let provider = get("llm_provider").unwrap_or_else(|| "openrouter".to_string());
        let api_key = crate::mode2::keyring_get(&provider).ok()?;
        return Some(crate::mode2::ProviderChoice::Api {
            provider,
            api_key,
            model: get("llm_model"),
            custom_base_url: get("llm_custom_url"),
        });
    }
    // Local: the active downloaded model (default = registry default).
    let model_id = get("local_model")
        .or_else(|| crate::mode2::models::REGISTRY.iter().find(|m| m.is_default).map(|m| m.id.to_string()))?;
    let app_data = app.path().app_data_dir().ok()?;
    let model_path = crate::mode2::models::model_path(&app_data, &model_id);
    if !model_path.exists() {
        return None; // not downloaded yet — stay on the panic card
    }
    let engine = app.state::<std::sync::Arc<crate::mode2::local::LocalEngine>>().inner().clone();
    Some(crate::mode2::ProviderChoice::Local {
        engine,
        model_id,
        model_path,
    })
}

fn gather_material(conn: &rusqlite::Connection) -> Result<crate::mode2::Material, String> {
    let mut stmt = conn
        .prepare(
            "SELECT b.text FROM bullets b JOIN cards c ON b.card_id = c.id
             WHERE c.source = 'prepared' ORDER BY c.created_at, b.position",
        )
        .map_err(|e| e.to_string())?;
    let bullets: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(crate::mode2::Material {
        corpus_bullets: bullets,
        ..Default::default()
    })
}

/// Phase-3 scratch session so card_events/coverage FKs hold before Phase 6.
pub fn ensure_scratch_session(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id, title, kind, status, language, created_at)
         VALUES (?1, 'Scratch (pre-sessions)', 'other', 'live', 'en', strftime('%s','now'))",
        params![SCRATCH_SESSION],
    )?;
    Ok(())
}
