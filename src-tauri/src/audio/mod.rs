//! Audio pipeline (Phase 4): dual-channel capture → streaming ASR → the same
//! `feed_transcript` path the Phase-3 transcript player used. Their speech
//! drives card selection; your speech drives coverage.

pub mod aec;
pub mod asr;
pub mod asr_models;
pub mod asr_offline;
pub mod capture;
pub mod engine;

use asr::Emit;
use capture::{AudioChunk, Channel};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

/// A "them" channel that stays silent this long while listening is almost
/// always a wrong/dead output endpoint — the classic earbuds/loopback trap.
const DEAD_CHANNEL_SECS: u64 = 6;

/// The mic delivers audio chunks CONTINUOUSLY (even silence), so if it stops
/// delivering for this long the stream itself died — unplug, mute, or a capture
/// rebuild that never recovered — as opposed to the user merely being quiet
/// (which still delivers near-zero chunks). This catches a mic that dies AFTER
/// producing audio, which the energy-only "never alive" check cannot see.
/// (The loopback "them" channel has real silence gaps during far-end pauses, so
/// delivery-based death detection there needs per-device knowledge — left to the
/// live audio pass; see Documents/11_AUDIT_2026-07-29.md.)
const MIC_STALL_SECS: u64 = 4;

/// Speaker-echo filter window and threshold: a "me" final that overlaps a
/// "them" final from the last ~4 s by this fraction of words is the mic
/// hearing the speakers, not the user — drop it from the match engine.
const ECHO_WINDOW_MS: u64 = 4000;
const ECHO_OVERLAP: f64 = 0.6;

/// Word-overlap of `a` relative to the shorter of the two (0..1). Cheap
/// bag-of-words containment — good enough to spot near-duplicate echoes
/// without flagging a user who merely says a couple of the same words.
/// Also reused by the Mode-2 debounce (live.rs).
pub fn text_overlap(a: &str, b: &str) -> f64 {
    let norm = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 1)
            .map(|w| w.to_lowercase())
            .collect()
    };
    let wa = norm(a);
    let wb = norm(b);
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let set_b: std::collections::HashSet<&String> = wb.iter().collect();
    let shared = wa.iter().filter(|w| set_b.contains(w)).count();
    shared as f64 / wa.len().min(wb.len()) as f64
}

/// Owns the running capture + worker; None when audio is stopped.
#[derive(Default)]
pub struct AudioState {
    running: Mutex<Option<Running>>,
}

struct Running {
    handles: capture::CaptureHandles,
    stop_worker: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    health: Option<std::thread::JoinHandle<()>>,
    /// The ASR language the worker was spawned with. If a later start_audio has a
    /// different language (pre-flight "auto" → a session's fixed language on
    /// go-live), the worker must be restarted to re-bind it.
    language: String,
}

/// Stop the capture + worker + watchdog if running (no status event — callers
/// that need one emit it). Shared by stop_audio and the language-restart path.
fn stop_running(audio: &AudioState) {
    let running = audio.running.lock().ok().and_then(|mut g| g.take());
    if let Some(mut r) = running {
        r.stop_worker.store(true, std::sync::atomic::Ordering::SeqCst);
        r.handles.stop(); // drops the capture tx → worker's rx closes
        if let Some(w) = r.worker.take() {
            let _ = w.join();
        }
        if let Some(h) = r.health.take() {
            let _ = h.join();
        }
    }
}

#[derive(Serialize, Clone)]
struct AsrStatus {
    running: bool,
    model: Option<String>,
    error: Option<String>,
}

#[derive(Serialize, Clone)]
struct PartialEvent {
    speaker: &'static str,
    text: String,
    final_: bool,
}

#[derive(Serialize, Clone)]
struct ChannelHealth {
    them_silent: bool,
    me_silent: bool,
}

#[tauri::command]
pub fn audio_status(audio: tauri::State<'_, AudioState>) -> bool {
    audio.running.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// The ASR language for this call, from the active session's `sessions.language`.
/// Scratch (no real session) or a blank/unknown value → "auto" (the multilingual
/// model detects the language itself). A DB-lock failure also degrades to "auto"
/// rather than blocking the call.
fn call_language(app: &tauri::AppHandle) -> String {
    let session = app.state::<crate::live::LiveState>().active_session_id();
    if session == crate::live::SCRATCH_SESSION {
        return "auto".to_string();
    }
    app.state::<crate::Db>()
        .conn
        .lock()
        .ok()
        .and_then(|conn| crate::store::session_language(&conn, &session))
        .unwrap_or_else(|| "auto".to_string())
}

#[tauri::command]
pub fn start_audio(app: tauri::AppHandle, audio: tauri::State<'_, AudioState>) -> Result<(), String> {
    // Language for this call (cheap DB read), computed up front so we can tell a
    // no-op start (already running with the RIGHT language) from a restart:
    // pre-flight starts capture as "auto", so when a fixed-language session goes
    // live the worker must be torn down and re-spawned bound to that language.
    let language = call_language(&app);
    {
        let guard = audio.running.lock().map_err(|e| e.to_string())?;
        if let Some(r) = guard.as_ref() {
            if r.language == language {
                return Ok(()); // already running with the right language
            }
        }
    }
    // Running with a DIFFERENT language (or, harmlessly, not running at all) →
    // tear it down before the fresh start so the worker re-binds the language.
    stop_running(audio.inner());

    // The ASR model load reads ~630 MB from disk over several seconds — do it
    // BEFORE taking the state lock so stop/status stay responsive during start.
    // Portable data folder, consistent with every other model path.
    let data_dir = crate::paths::data_dir();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(8);
    // Engine choice: env override, else the `asr_engine` setting, else Auto
    // (streaming primary when present, else the offline Parakeet fallback).
    let pref = std::env::var("ANCHOR_ASR_ENGINE")
        .ok()
        .or_else(|| {
            app.state::<crate::Db>()
                .conn
                .lock()
                .ok()
                .and_then(|c| crate::setting_get(&c, "asr_engine"))
        })
        .map(|s| engine::EnginePref::parse(&s))
        .unwrap_or(engine::EnginePref::Auto);
    let asr = engine::AsrEngine::load(&data_dir, threads, pref)?;
    // `language` (computed up front) steers the multilingual model.
    tracing::info!(engine = asr.label(), language = %language, "ASR engine for the call");

    let mut guard = audio.running.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(()); // lost a start/start race — drop the just-loaded model
    }

    let (tx, rx) = std::sync::mpsc::channel::<AudioChunk>();
    let handles = capture::start(tx)?;

    let stop_worker = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Millis-since-start of the last LIVE (energy) sample on each channel (0 = never).
    let them_seen = Arc::new(AtomicU64::new(0));
    let me_seen = Arc::new(AtomicU64::new(0));
    // Last mic chunk DELIVERY time (0 = never), regardless of energy — used to
    // detect a mic that dies mid-call (see mic_stream_stalled).
    let me_delivered = Arc::new(AtomicU64::new(0));
    let model = Some(asr.label().to_string()); // captured before `asr` moves into the worker
    let worker = {
        let app = app.clone();
        let stop = stop_worker.clone();
        let them_seen = them_seen.clone();
        let me_seen = me_seen.clone();
        let me_delivered = me_delivered.clone();
        let language = language.clone(); // keep the original for `Running.language`
        std::thread::spawn(move || {
            worker_loop(app, asr, language, rx, stop, them_seen, me_seen, me_delivered)
        })
    };
    // Health watchdog: warns when a channel goes quiet (wrong endpoint / earbuds).
    let health = {
        let app = app.clone();
        let stop = stop_worker.clone();
        std::thread::spawn(move || health_loop(app, stop, them_seen, me_seen, me_delivered))
    };

    *guard = Some(Running {
        handles,
        stop_worker,
        worker: Some(worker),
        health: Some(health),
        language,
    });
    app.emit_to(
        "dashboard",
        "audio:status",
        AsrStatus {
            running: true,
            model,
            error: None,
        },
    )
    .ok();
    tracing::info!("audio started");
    Ok(())
}

#[tauri::command]
pub fn stop_audio(app: tauri::AppHandle, audio: tauri::State<'_, AudioState>) -> Result<(), String> {
    // Joins the worker + watchdog, so stop returns only once every thread is
    // down — no stale health event can fire after stop or into a restart.
    stop_running(audio.inner());
    app.emit_to(
        "dashboard",
        "audio:status",
        AsrStatus {
            running: false,
            model: None,
            error: None,
        },
    )
    .ok();
    tracing::info!("audio stopped");
    Ok(())
}

/// Drains audio chunks, runs ASR per channel, and forwards results:
/// One-shot audio diagnostic for the live call test: do the two capture channels
/// share a timeline? If the loopback device reports bad QPC timestamps, "them"
/// anchors far from "me" and the signal-level AEC silently cancels nothing (H1).
/// Records the first samples' timestamps per channel and writes a summary to
/// `data_dir/audio-diag.txt` for the owner to send back. Cheap: first 30 chunks
/// each, then a single file write.
#[derive(Default)]
struct AudioDiag {
    them_first: Option<u64>,
    them_last: u64,
    them_n: u32,
    me_first: Option<u64>,
    me_last: u64,
    me_n: u32,
    written: bool,
}

impl AudioDiag {
    fn record(&mut self, ch: Channel, ts_us: u64) {
        let (first, last, n) = match ch {
            Channel::Them => (&mut self.them_first, &mut self.them_last, &mut self.them_n),
            Channel::Me => (&mut self.me_first, &mut self.me_last, &mut self.me_n),
        };
        if first.is_none() {
            *first = Some(ts_us);
        }
        *last = ts_us;
        *n += 1;
    }

    fn maybe_write(&mut self) {
        if self.written || self.them_n < 30 || self.me_n < 30 {
            return;
        }
        self.written = true;
        let them0 = self.them_first.unwrap_or(0);
        let me0 = self.me_first.unwrap_or(0);
        let gap_ms = (them0 as i64 - me0 as i64).abs() / 1000;
        let aligned = gap_ms < 3000;
        let body = format!(
            "Anchor audio diagnostic\n\
             them (system audio): {} chunks, first ts_us={them0}, last ts_us={}\n\
             me   (microphone):   {} chunks, first ts_us={me0}, last ts_us={}\n\
             timeline gap |them_first - me_first| = {gap_ms} ms\n\
             AEC timeline alignment: {}\n\
             (MISALIGNED => the loopback device gives bad timestamps; signal-level\n\
              echo cancellation is not working on this hardware — finding H1.)\n",
            self.them_n,
            self.them_last,
            self.me_n,
            self.me_last,
            if aligned { "ALIGNED (ok)" } else { "MISALIGNED (H1 suspected)" },
        );
        let path = crate::paths::data_dir().join("audio-diag.txt");
        let _ = std::fs::write(path, body);
    }
}

/// finals go into the rolling windows (match engine); partials go to the UI.
#[allow(clippy::too_many_arguments)] // cohesive thread-entry args (asr + 3 liveness atomics)
fn worker_loop(
    app: tauri::AppHandle,
    asr: engine::AsrEngine,
    language: String,
    rx: Receiver<AudioChunk>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    them_seen: Arc<AtomicU64>,
    me_seen: Arc<AtomicU64>,
    me_delivered: Arc<AtomicU64>,
) {
    // Both channels of a call share the session's language; "auto" (scratch /
    // no session) lets the multilingual model detect it per channel.
    let mut them = asr.new_channel(&language);
    let mut me = asr.new_channel(&language);
    let origin = Instant::now();
    // Recent "them" finals, for the speaker-echo filter (see below).
    let mut recent_them: std::collections::VecDeque<(u64, String)> = std::collections::VecDeque::new();
    // Signal-level AEC: "them" is the time-aligned reference; "me" is cleaned
    // before ASR so the far side's voice (heard from open speakers) is not
    // transcribed on the mic. Harmless with headphones (no echo to cancel).
    let mut echo_canceller = aec::EchoCanceller::new();
    let mut diag = AudioDiag::default();

    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let chunk = match rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let now_ms = origin.elapsed().as_millis() as u64;
        // Record mic chunk DELIVERY (energy-independent) — a gap means the stream
        // died, which the energy check below cannot distinguish from a quiet user.
        if matches!(chunk.channel, Channel::Me) {
            me_delivered.store(now_ms.max(1), Ordering::SeqCst);
        }
        // One-shot channel-timeline diagnostic for the live test (H1).
        diag.record(chunk.channel, chunk.ts_us);
        diag.maybe_write();
        // Liveness uses the RAW device audio (is it delivering anything at all),
        // before AEC — the cancelled mic can be near-silent when it's all echo.
        let raw_alive = chunk.samples.iter().any(|s| s.abs() > 1e-4);
        // AEC: buffer "them" as the reference; clean "me" against it.
        if chunk.channel == Channel::Them {
            echo_canceller.push_reference(chunk.ts_us, &chunk.samples);
        }
        let me_cleaned = match chunk.channel {
            Channel::Me => Some(echo_canceller.push_mic(chunk.ts_us, &chunk.samples)),
            Channel::Them => None,
        };
        let (speaker, ch, seen) = match chunk.channel {
            Channel::Them => ("them", &mut them, &them_seen),
            Channel::Me => ("me", &mut me, &me_seen),
        };
        if raw_alive {
            seen.store(now_ms.max(1), Ordering::SeqCst);
        }
        let samples: &[f32] = match &me_cleaned {
            Some(cleaned) => cleaned, // echo-cancelled mic (may be empty until a frame + lag is buffered)
            None => &chunk.samples,   // "them" loopback, fed raw
        };
        if samples.is_empty() {
            continue; // mic frame not ready yet — held in the canceller
        }
        let emit = asr.feed(ch, chunk.sample_rate, samples);
        match emit {
            Emit::Final(text) => {
                if speaker == "them" {
                    recent_them.push_back((now_ms, text.clone()));
                    while recent_them.front().is_some_and(|(t, _)| now_ms - t > ECHO_WINDOW_MS) {
                        recent_them.pop_front();
                    }
                }
                // Speaker-echo filter: on open speakers (no headphones) the mic
                // hears the other side out loud, so "them" bleeds into "me" as a
                // near-identical final. Feeding that to the ME window would mark
                // bullets covered while the OTHER person is talking. Drop it from
                // the engine (still show the partial). Full AEC is Phase 7.
                //
                // The age check is here, not only on push: a "them" final that
                // is never followed by another would otherwise linger forever
                // and wrongly flag a user answer that reuses the question's
                // words minutes later.
                let is_echo = speaker == "me"
                    && recent_them.iter().any(|(t, txt)| {
                        now_ms.saturating_sub(*t) <= ECHO_WINDOW_MS
                            && text_overlap(&text, txt) >= ECHO_OVERLAP
                    });

                if !is_echo {
                    // Same path the Phase-3 player used → the match engine ticks,
                    // and the confirmed line lands in the session transcript.
                    if let Err(e) = crate::live::feed_and_persist(&app, speaker, &text) {
                        tracing::warn!(error = %e, "feed_transcript failed");
                    }
                } else {
                    tracing::debug!(text = %text, "dropped mic echo of system audio");
                }
                app.emit_to(
                    "dashboard",
                    "asr:partial",
                    PartialEvent {
                        speaker: if speaker == "them" { "them" } else { "me" },
                        text,
                        final_: true,
                    },
                )
                .ok();
            }
            Emit::Pending(text) => {
                app.emit_to(
                    "dashboard",
                    "asr:partial",
                    PartialEvent {
                        speaker: if speaker == "them" { "them" } else { "me" },
                        text,
                        final_: false,
                    },
                )
                .ok();
            }
            Emit::Nothing => {}
        }
    }
    tracing::debug!("audio worker exited");
}

/// A channel counts as a wiring problem only if it has produced NO live audio
/// since the check began (`last_live_ms == 0`) and the startup grace has passed.
/// Rolling silence is NOT death: loopback is silent for whole far-end turns and
/// the mic is quiet while the user listens. `last_live_ms` is 0 until the first
/// live sample (stored as `now_ms.max(1)`).
fn channel_is_dead(last_live_ms: u64, now_ms: u64) -> bool {
    last_live_ms == 0 && now_ms > DEAD_CHANNEL_SECS * 1000
}

/// The mic stopped DELIVERING chunks after having delivered some — a dead stream
/// (unplug / failed rebuild), not just a quiet user. `last_delivery_ms` is 0
/// until the first chunk, so a never-started mic is left to `channel_is_dead`.
fn mic_stream_stalled(last_delivery_ms: u64, now_ms: u64) -> bool {
    last_delivery_ms != 0 && now_ms.saturating_sub(last_delivery_ms) > MIC_STALL_SECS * 1000
}

/// Emits a health event whenever a channel's silence state changes.
fn health_loop(
    app: tauri::AppHandle,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    them_seen: Arc<AtomicU64>,
    me_seen: Arc<AtomicU64>,
    me_delivered: Arc<AtomicU64>,
) {
    let origin = Instant::now();
    let mut last: Option<ChannelHealth> = None;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1000));
        let now_ms = origin.elapsed().as_millis() as u64;
        // A channel is a wiring problem if it has produced NO live audio at all
        // since the check began (past a short grace), NOT if it fell silent for a
        // moment. Loopback ("them") emits nothing for whole far-end turns by
        // design, and the mic ("me") is quiet while the user listens — rolling
        // silence is normal on a real, turn-taking call, so treating it as death
        // fired a false "dead output" alarm on essentially every turn. `seen` is
        // 0 until the first live sample (stored as now_ms.max(1)), so 0 == never.
        let dead = |seen: &AtomicU64| channel_is_dead(seen.load(Ordering::SeqCst), now_ms);
        let health = ChannelHealth {
            them_silent: dead(&them_seen),
            // The mic ALSO counts as silent if its stream stopped delivering
            // mid-call (dead-after-alive), which energy-only liveness misses.
            me_silent: dead(&me_seen)
                || mic_stream_stalled(me_delivered.load(Ordering::SeqCst), now_ms),
        };
        if last.as_ref().map(|l| (l.them_silent, l.me_silent))
            != Some((health.them_silent, health.me_silent))
        {
            app.emit_to("dashboard", "audio:health", health.clone()).ok();
            last = Some(health);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_of_the_other_side_is_detected() {
        // Mic hears the speakers: near-identical text → flagged as echo.
        let them = "so tell me why are you leaving your own company";
        let me_echo = "so tell me why are you leaving your own company";
        assert!(text_overlap(me_echo, them) >= ECHO_OVERLAP);
    }

    #[test]
    fn the_users_own_distinct_answer_is_not_echo() {
        let them = "so tell me why are you leaving your own company";
        let me_answer = "good question i love building things and want more depth";
        assert!(text_overlap(me_answer, them) < ECHO_OVERLAP);
    }

    #[test]
    fn a_few_shared_words_do_not_trip_the_filter() {
        let them = "what are your salary expectations for this role";
        let me = "my salary target is competitive and flexible";
        assert!(text_overlap(me, them) < ECHO_OVERLAP);
    }

    #[test]
    fn rolling_silence_is_not_a_dead_channel() {
        // Regression (audit 2026-07-28): the old check was `now_ms - seen > T`,
        // which flagged a channel that spoke once and then went quiet — i.e. a
        // normal far-end turn — as a dead output on EVERY turn.
        let grace = DEAD_CHANNEL_SECS * 1000;
        // Truly never-live, past the grace → dead (the real wiring problem).
        assert!(channel_is_dead(0, grace + 1));
        // Never-live but still within the startup grace → not yet flagged.
        assert!(!channel_is_dead(0, grace - 1));
        // Produced audio early (seen != 0), then silent for a long time: MUST
        // NOT be dead. The old rolling-silence logic wrongly flagged this.
        assert!(!channel_is_dead(1, grace + 60_000));
        assert!(!channel_is_dead(500, 10 * grace));
    }

    #[test]
    fn mic_stall_is_a_dead_stream_not_a_quiet_user() {
        // Delivery is energy-INDEPENDENT: a quiet-but-alive mic still delivers
        // (near-silent) chunks, so it must NOT be flagged; only a mic that stops
        // delivering entirely (unplug / failed rebuild) is stalled.
        let t = MIC_STALL_SECS * 1000;
        assert!(!mic_stream_stalled(0, t + 5000)); // never delivered → channel_is_dead's job
        assert!(!mic_stream_stalled(9_000, 9_000 + t - 1)); // delivered recently → alive
        assert!(mic_stream_stalled(1_000, 1_000 + t + 1)); // delivered, then gone → dead
    }
}
