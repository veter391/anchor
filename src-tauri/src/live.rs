//! The live session runtime: transcript feed → rolling windows → ticker →
//! match engine → events to the overlay and the debug panel.
//!
//! THEIR speech drives card selection; YOUR speech drives bullet coverage
//! (02_ARCHITECTURE §5). Every tick: embed outside the DB lock, then one
//! short lock for search + logging. Jumps are logged to card_events with
//! the runner-up — that log is the threshold-tuning dataset.

use crate::matcher::{Candidate, Decision, MatchEngine, RollingWindow, Thresholds};
use crate::{search, store};
use rusqlite::{params, OptionalExtension};
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
    /// The session the live loop is bound to right now. Defaults to the scratch
    /// session (the Phase-3 dev harness — retrieval stays global there, exactly
    /// as verified). Set to a real session id by `set_active_session`, which
    /// scopes retrieval, coverage and card_events to that session's own cards.
    active_session: Mutex<String>,
    dirty: AtomicBool,
    /// Mode-2 debounce: true while an assembly is in flight, plus the
    /// question text we last assembled for (so we don't re-fire every tick
    /// while the same unmatched question sits in the window).
    mode2_inflight: AtomicBool,
    mode2_last_q: Mutex<String>,
    /// Bumped by reset_live; an in-flight assembly from a previous epoch
    /// must not push its stale card onto the freshly-reset overlay.
    mode2_epoch: std::sync::atomic::AtomicU64,
    /// True while an assembled/panic card is covering the matched card. The
    /// match engine's `current_card` does NOT change when Mode-2 fires, so once
    /// the engine regains confidence in the *same* card it returns `Stay` and
    /// emits nothing — leaving the panic card stuck on screen. This flag lets a
    /// `Stay` (or `Jump`) know it must bring the real card back.
    overlay_off_card: AtomicBool,
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
            active_session: Mutex::new(SCRATCH_SESSION.to_string()),
            dirty: AtomicBool::new(false),
            mode2_inflight: AtomicBool::new(false),
            mode2_last_q: Mutex::new(String::new()),
            mode2_epoch: std::sync::atomic::AtomicU64::new(0),
            overlay_off_card: AtomicBool::new(false),
        }
    }
}

fn lock_or_recover<'a, T>(m: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Re-emits the current card in the active bullet-length style. Used when
/// the style setting changes so the overlay restyles in the moment.
pub(crate) fn restyle_current_card(app: &tauri::AppHandle) -> Result<(), String> {
    let live = app.state::<LiveState>();
    let card_id = {
        let engine = lock_or_recover(&live.engine);
        engine.current_card().map(String::from)
    };
    let Some(card_id) = card_id else { return Ok(()) };
    let db = app.state::<crate::Db>();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    if let Some(card) = store::get_card(&conn, &card_id)? {
        app.emit_to("overlay", "card:show", &card).ok();
    }
    Ok(())
}

// ── Commands ────────────────────────────────────────────────────────

#[tauri::command]
pub fn feed_transcript(
    app: tauri::AppHandle,
    speaker: String,
    text: String,
) -> Result<(), String> {
    feed_and_persist(&app, &speaker, &text)
}

/// Push a confirmed line into the rolling windows; returns the timeline ts (ms).
pub fn feed_transcript_internal(
    live: &LiveState,
    speaker: &str,
    text: &str,
) -> Result<u64, String> {
    let mut w = lock_or_recover(&live.windows);
    let ts_ms = w.origin.elapsed().as_millis() as u64;
    match speaker {
        "them" => w.them.push(ts_ms, text.to_string()),
        "me" => w.me.push(ts_ms, text.to_string()),
        other => return Err(format!("unknown speaker: {other}")),
    }
    live.dirty.store(true, Ordering::SeqCst);
    Ok(ts_ms)
}

/// Feed a confirmed line AND persist it to the ACTIVE session's transcript
/// (text only — audio never touches disk, 08_LEGAL). The scratch dev harness is
/// never persisted. Shared by the audio worker and the manual feed command.
pub fn feed_and_persist(app: &tauri::AppHandle, speaker: &str, text: &str) -> Result<(), String> {
    let live = app.state::<LiveState>();
    let ts_ms = feed_transcript_internal(&live, speaker, text)?;
    let session = lock_or_recover(&live.active_session).clone();
    if session != SCRATCH_SESSION {
        let db = app.state::<crate::Db>();
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO transcript (session_id, speaker, ts_ms, text)
             VALUES (?1, ?2, ?3, ?4)",
            params![session, speaker, ts_ms as i64, text],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn reset_live(
    app: tauri::AppHandle,
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
    live.mode2_epoch.fetch_add(1, Ordering::SeqCst);
    live.overlay_off_card.store(false, Ordering::SeqCst);
    lock_or_recover(&live.mode2_last_q).clear();
    // Clear the ACTIVE session's live rows so a re-run doesn't accumulate
    // duplicate coverage/event rows (coverage is sticky in memory; the DB is
    // the record, and the engine reset just wiped the in-memory flags).
    let session = lock_or_recover(&live.active_session).clone();
    let conn = db.conn.lock().unwrap_or_else(|p| p.into_inner());
    conn.execute(
        "DELETE FROM coverage WHERE session_id = ?1",
        rusqlite::params![session],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM card_events WHERE session_id = ?1",
        rusqlite::params![session],
    )
    .map_err(|e| e.to_string())?;
    live.dirty.store(true, Ordering::SeqCst);
    app.emit_to("overlay", "live:cleared", ()).ok();
    Ok(())
}

/// Bind the live loop to a real session: retrieval, coverage and card_events
/// now scope to THIS session's own cards (02_ARCHITECTURE §5, 05_DATA_MODEL).
/// Resets the windows + engine for a clean start and marks the session live.
#[tauri::command]
pub fn set_active_session(
    app: tauri::AppHandle,
    live: tauri::State<'_, LiveState>,
    db: tauri::State<'_, crate::Db>,
    session_id: String,
) -> Result<(), String> {
    if session_id == SCRATCH_SESSION {
        return Err("not a real session".into());
    }
    let prev = lock_or_recover(&live.active_session).clone();
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM sessions WHERE id = ?1",
                params![session_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .is_some();
        if !exists {
            return Err("session not found".into());
        }
        // Going live on a new session demotes a previously-live one back to
        // planned (no session is left stuck showing the live dot forever).
        if prev != SCRATCH_SESSION && prev != session_id {
            conn.execute(
                "UPDATE sessions SET status = 'planned' WHERE id = ?1 AND status = 'live'",
                params![prev],
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute(
            "UPDATE sessions SET status = 'live',
                 started_at = COALESCE(started_at, strftime('%s','now'))
             WHERE id = ?1",
            params![session_id],
        )
        .map_err(|e| e.to_string())?;
    }
    reset_engine(&live);
    *lock_or_recover(&live.active_session) = session_id;
    live.dirty.store(true, Ordering::SeqCst);
    app.emit_to("overlay", "live:cleared", ()).ok();
    Ok(())
}

/// Stop the call: unbind from the session (revert to the scratch harness) and
/// mark it planned again. The green/red close + coverage report is the next
/// Phase-6 step; this is the plain "stop listening".
#[tauri::command]
pub fn clear_active_session(
    app: tauri::AppHandle,
    live: tauri::State<'_, LiveState>,
    db: tauri::State<'_, crate::Db>,
) -> Result<(), String> {
    let prev = {
        let mut a = lock_or_recover(&live.active_session);
        let prev = a.clone();
        *a = SCRATCH_SESSION.to_string();
        prev
    };
    if prev != SCRATCH_SESSION {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET status = 'planned' WHERE id = ?1 AND status = 'live'",
            params![prev],
        )
        .map_err(|e| e.to_string())?;
    }
    reset_engine(&live);
    live.dirty.store(true, Ordering::SeqCst);
    app.emit_to("overlay", "live:cleared", ()).ok();
    Ok(())
}

/// The session the live loop is bound to right now (`scratch` = the dev harness,
/// i.e. no real session live).
#[tauri::command]
pub fn get_active_session(live: tauri::State<'_, LiveState>) -> String {
    lock_or_recover(&live.active_session).clone()
}

impl LiveState {
    /// The active session id, for non-command callers (e.g. the audio worker
    /// resolving the session's ASR language). `scratch` = no real session.
    pub fn active_session_id(&self) -> String {
        lock_or_recover(&self.active_session).clone()
    }
}

/// Fresh windows + engine + Mode-2 debounce — a clean slate for a new call.
fn reset_engine(live: &LiveState) {
    {
        let mut w = lock_or_recover(&live.windows);
        w.them.clear();
        w.me.clear();
        w.origin = Instant::now();
    }
    lock_or_recover(&live.engine).reset();
    live.mode2_inflight.store(false, Ordering::SeqCst);
    live.mode2_epoch.fetch_add(1, Ordering::SeqCst);
    live.overlay_off_card.store(false, Ordering::SeqCst);
    lock_or_recover(&live.mode2_last_q).clear();
}

/// End the call: build the coverage report ("what you failed to say"), mark the
/// session closed green/red, and unbind the live loop if it was this session.
#[tauri::command]
pub fn close_session(
    app: tauri::AppHandle,
    live: tauri::State<'_, LiveState>,
    db: tauri::State<'_, crate::Db>,
    session_id: String,
) -> Result<store::CoverageReport, String> {
    let report = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let report = store::coverage_report(&conn, &session_id)?;
        let status = if report.verdict == "green" {
            "closed_green"
        } else {
            "closed_red"
        };
        conn.execute(
            "UPDATE sessions SET status = ?2, closed_at = strftime('%s','now') WHERE id = ?1",
            params![session_id, status],
        )
        .map_err(|e| e.to_string())?;
        report
    };
    // If the call we're ending is the live one, unbind and clear the overlay.
    let was_active = *lock_or_recover(&live.active_session) == session_id;
    if was_active {
        *lock_or_recover(&live.active_session) = SCRATCH_SESSION.to_string();
        reset_engine(&live);
        live.dirty.store(true, Ordering::SeqCst);
        app.emit_to("overlay", "live:cleared", ()).ok();
    }
    Ok(report)
}

/// Read-only coverage report for a session (to view a closed session later).
#[tauri::command]
pub fn session_report(
    db: tauri::State<'_, crate::Db>,
    session_id: String,
) -> Result<store::CoverageReport, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    store::coverage_report(&conn, &session_id)
}

/// Reopen a closed session for another run: back to planned, old coverage and
/// events wiped so the next call's report starts clean.
#[tauri::command]
pub fn reopen_session(
    db: tauri::State<'_, crate::Db>,
    session_id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE sessions SET status = 'planned', closed_at = NULL WHERE id = ?1",
        params![session_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM coverage WHERE session_id = ?1",
        params![session_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM card_events WHERE session_id = ?1",
        params![session_id],
    )
    .map_err(|e| e.to_string())?;
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
    // The session this call is bound to — scopes retrieval, coverage and events.
    let session = lock_or_recover(&live.active_session).clone();

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
        // Scratch (dev harness) keeps the global, already-verified retrieval;
        // a real session retrieves only against its own cards.
        let matches = if session == SCRATCH_SESSION {
            search::query_cards_with_vec(&conn, qvec, &them_text)?
        } else {
            search::query_cards_scoped(&conn, qvec, &them_text, &session)?
        };
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
                        // Always-on guard: a dependency bump changing the metric
                        // must scream in release builds too (audit 2026-07-23).
                        if !(0.0..=2.001).contains(&d) {
                            tracing::error!(d, "vec distance outside L2 range — metric changed?");
                        }
                        1.0 - (d * d) / 2.0
                    })
                    // Keyword-only hits: sub-order by real BM25 rank so ties are
                    // deterministic, not HashMap-iteration luck (audit 2026-07-23).
                    .unwrap_or_else(|| {
                        BM25_ONLY_SCORE - m.bm25_rank.unwrap_or(50) as f64 * 1e-3
                    }),
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
                            session.as_str(),
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
                    // A real card is going on screen: dismiss any panic/assembled
                    // card and supersede an in-flight assembly so it can't clobber
                    // this jump when it lands.
                    if live.overlay_off_card.swap(false, Ordering::SeqCst) {
                        live.mode2_epoch.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
            Decision::NoConfidence => {
                no_confidence = true;
                maybe_assemble(app, &live, &conn, &them_text, &session);
            }
            Decision::Stay => {
                // The engine is confident in the SAME card again. If a panic/
                // assembled card is currently covering it, bring the real card
                // back (Stay emits nothing on its own) and cancel any pending
                // assembly for the now-resolved question.
                if live.overlay_off_card.swap(false, Ordering::SeqCst) {
                    live.mode2_epoch.fetch_add(1, Ordering::SeqCst);
                    let cur = lock_or_recover(&live.engine).current_card().map(String::from);
                    if let Some(cid) = cur {
                        if let Some(card) = store::get_card(&conn, &cid)? {
                            jumped = Some(card);
                        }
                    }
                }
            }
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
                            // Coverage is sticky; update_coverage returns the full
                            // flag vector, so guard against re-inserting a bullet
                            // that is already marked covered (no duplicate rows).
                            "INSERT INTO coverage (session_id, card_id, bullet_id, covered, score, ts_ms)
                             SELECT ?1, ?2, ?3, 1, NULL, ?4
                             WHERE NOT EXISTS (
                               SELECT 1 FROM coverage
                               WHERE session_id = ?1 AND bullet_id = ?3 AND covered = 1
                             )",
                            params![session.as_str(), card_id, bullet_id, ts_ms],
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
    session: &str,
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

    // Gather the user's material (the session's cards, or all prepared bullets
    // on the scratch dev path) while we hold the lock.
    let material = match gather_material(conn, session) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "mode2: gather material failed");
            return;
        }
    };

    live.mode2_inflight.store(true, Ordering::SeqCst);
    *lock_or_recover(&live.mode2_last_q) = q.to_string();

    // Panic card instantly, as the filler while assembly runs. From now the
    // overlay shows something other than the matched card, until a Jump/Stay
    // brings the real card back (see LiveState::overlay_off_card).
    let panic = crate::mode2::panic_card();
    live.overlay_off_card.store(true, Ordering::SeqCst);
    app.emit_to("overlay", "card:assembled", &panic).ok();

    let app2 = app.clone();
    let embedder = app.state::<std::sync::Arc<crate::embed::Embedder>>().inner().clone();
    let question = q.to_string();
    let epoch = live.mode2_epoch.load(Ordering::SeqCst);
    // Off the ticker thread: network + embedding, then emit the result.
    tauri::async_runtime::spawn(async move {
        let result =
            crate::mode2::assemble(&choice, &embedder, &material, &question, &style).await;
        let live = app2.state::<LiveState>();
        // A reset_live during assembly supersedes this run — never push a
        // stale card onto the freshly-reset overlay (audit 2026-07-23).
        let stale = live.mode2_epoch.load(Ordering::SeqCst) != epoch;
        match result {
            Ok(card) if !stale => {
                app2.emit_to("overlay", "card:assembled", &card).ok();
                app2.emit_to("dashboard", "mode2:done", &card).ok();
            }
            Ok(_) => tracing::info!("mode2: discarding assembly from a superseded run"),
            Err(e) => {
                tracing::warn!(error = %e, "mode2 assembly failed");
                if !stale {
                    app2.emit_to("dashboard", "mode2:error", &e).ok();
                }
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
    // Local: the active downloaded model (default = registry default). Ignore a
    // stored id that is not in the registry (tampered DB) rather than join it
    // into a path — models::model_path below would otherwise traverse on `..\`.
    let model_id = get("local_model")
        .filter(|id| crate::mode2::models::find(id).is_some())
        .or_else(|| crate::mode2::models::REGISTRY.iter().find(|m| m.is_default).map(|m| m.id.to_string()))?;
    // Portable data folder — MUST match list/download/delete_model, else a
    // freshly-downloaded model isn't found here and Local mode silently fails.
    let data_dir = crate::paths::data_dir();
    let model_path = crate::mode2::models::model_path(&data_dir, &model_id);
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

fn gather_material(
    conn: &rusqlite::Connection,
    session: &str,
) -> Result<crate::mode2::Material, String> {
    // Bullets WITH their stored embeddings: assembly ranks them by relevance
    // to the question (prompt stays inside the local model's context however
    // big the corpus is) and the post-check reuses the vectors instead of
    // re-embedding the whole corpus per fire (audit 2026-07-23). A real session
    // grounds only in its own cards; the scratch dev path uses all prepared.
    let sql = if session == SCRATCH_SESSION {
        "SELECT b.text, v.embedding FROM bullets b
         JOIN cards c ON b.card_id = c.id
         LEFT JOIN bullet_vec v ON v.bullet_id = b.id
         WHERE c.source = 'prepared' ORDER BY c.created_at, b.position"
    } else {
        "SELECT b.text, v.embedding FROM bullets b
         JOIN cards c ON b.card_id = c.id
         LEFT JOIN bullet_vec v ON v.bullet_id = b.id
         WHERE c.session_id = ?1 ORDER BY c.created_at, b.position"
    };
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let row_fn =
        |r: &rusqlite::Row| Ok((r.get::<_, String>(0)?, r.get::<_, Option<Vec<u8>>>(1)?));
    let rows: Vec<(String, Option<Vec<u8>>)> = if session == SCRATCH_SESSION {
        stmt.query_map([], row_fn)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map(params![session], row_fn)
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect()
    };
    let mut bullets = Vec::new();
    let mut vecs = Vec::new();
    for (text, blob) in rows {
        bullets.push(text);
        vecs.push(blob.map(|b| crate::embed::blob_to_vec(&b)));
    }
    Ok(crate::mode2::Material {
        corpus_bullets: bullets,
        corpus_vecs: vecs,
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
