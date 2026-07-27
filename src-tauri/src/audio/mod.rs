//! Audio pipeline (Phase 4): dual-channel capture → streaming ASR → the same
//! `feed_transcript` path the Phase-3 transcript player used. Their speech
//! drives card selection; your speech drives coverage.

pub mod aec;
pub mod asr;
pub mod capture;

use asr::{Asr, Emit};
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
    // Cheap running-check without holding the lock across the model load.
    if audio.running.lock().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }

    // The ASR model load reads ~633 MB from disk over several seconds — do it
    // BEFORE taking the state lock so stop/status stay responsive during start.
    // Portable data folder, consistent with every other model path.
    let data_dir = crate::paths::data_dir();
    let model_dir = asr::model_dir(&data_dir)
        .ok_or("no ASR model found — set ANCHOR_ASR_MODEL_DIR or install the model")?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(8);
    let asr = Asr::load(&model_dir, threads)?;
    // Steer the multilingual model with the active session's expected language.
    let language = call_language(&app);
    tracing::info!(language = %language, "resolved ASR language for the call");

    let mut guard = audio.running.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(()); // lost a start/start race — drop the just-loaded model
    }

    let (tx, rx) = std::sync::mpsc::channel::<AudioChunk>();
    let handles = capture::start(tx)?;

    let stop_worker = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Millis-since-start of the last chunk seen on each channel (0 = never).
    let them_seen = Arc::new(AtomicU64::new(0));
    let me_seen = Arc::new(AtomicU64::new(0));
    let worker = {
        let app = app.clone();
        let stop = stop_worker.clone();
        let them_seen = them_seen.clone();
        let me_seen = me_seen.clone();
        std::thread::spawn(move || worker_loop(app, asr, language, rx, stop, them_seen, me_seen))
    };
    // Health watchdog: warns when a channel goes quiet (wrong endpoint / earbuds).
    let health = {
        let app = app.clone();
        let stop = stop_worker.clone();
        std::thread::spawn(move || health_loop(app, stop, them_seen, me_seen))
    };

    *guard = Some(Running {
        handles,
        stop_worker,
        worker: Some(worker),
        health: Some(health),
    });
    let model = model_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
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
    let running = audio.running.lock().map_err(|e| e.to_string())?.take();
    if let Some(mut r) = running {
        r.stop_worker
            .store(true, std::sync::atomic::Ordering::SeqCst);
        r.handles.stop(); // drops the capture tx → worker's rx closes
        if let Some(w) = r.worker.take() {
            let _ = w.join();
        }
        // Join the watchdog too, so stop_audio returns only once every thread
        // is down — no stale health event can fire after stop or into a restart.
        if let Some(h) = r.health.take() {
            let _ = h.join();
        }
    }
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
/// finals go into the rolling windows (match engine); partials go to the UI.
fn worker_loop(
    app: tauri::AppHandle,
    asr: Asr,
    language: String,
    rx: Receiver<AudioChunk>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    them_seen: Arc<AtomicU64>,
    me_seen: Arc<AtomicU64>,
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

    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let chunk = match rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let now_ms = origin.elapsed().as_millis() as u64;
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

/// Emits a health event whenever a channel's silence state changes.
fn health_loop(
    app: tauri::AppHandle,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    them_seen: Arc<AtomicU64>,
    me_seen: Arc<AtomicU64>,
) {
    let origin = Instant::now();
    let mut last: Option<ChannelHealth> = None;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1000));
        let now_ms = origin.elapsed().as_millis() as u64;
        let silent = |seen: &AtomicU64| {
            let s = seen.load(Ordering::SeqCst);
            // Never-seen or quiet for longer than the threshold.
            now_ms.saturating_sub(s) > DEAD_CHANNEL_SECS * 1000
        };
        let health = ChannelHealth {
            them_silent: silent(&them_seen),
            me_silent: silent(&me_seen),
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
}
