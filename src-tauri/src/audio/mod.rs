//! Audio pipeline (Phase 4): dual-channel capture → streaming ASR → the same
//! `feed_transcript` path the Phase-3 transcript player used. Their speech
//! drives card selection; your speech drives coverage.

pub mod asr;
pub mod capture;

use crate::live::LiveState;
use asr::{Asr, Emit};
use capture::{AudioChunk, Channel};
use serde::Serialize;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// Owns the running capture + worker; None when audio is stopped.
#[derive(Default)]
pub struct AudioState {
    running: Mutex<Option<Running>>,
}

struct Running {
    handles: capture::CaptureHandles,
    stop_worker: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
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

#[tauri::command]
pub fn audio_status(audio: tauri::State<'_, AudioState>) -> bool {
    audio.running.lock().map(|g| g.is_some()).unwrap_or(false)
}

#[tauri::command]
pub fn start_audio(app: tauri::AppHandle, audio: tauri::State<'_, AudioState>) -> Result<(), String> {
    let mut guard = audio.running.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(());
    }

    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let model_dir = asr::model_dir(&app_data)
        .ok_or("no ASR model found — set ANCHOR_ASR_MODEL_DIR or install the model")?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(8);
    let asr = Asr::load(&model_dir, threads)?;

    let (tx, rx) = std::sync::mpsc::channel::<AudioChunk>();
    let handles = capture::start(tx)?;

    let stop_worker = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker = {
        let app = app.clone();
        let stop = stop_worker.clone();
        std::thread::spawn(move || worker_loop(app, asr, rx, stop))
    };

    *guard = Some(Running {
        handles,
        stop_worker,
        worker: Some(worker),
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
    rx: Receiver<AudioChunk>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut them = asr.new_channel();
    let mut me = asr.new_channel();

    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let chunk = match rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let (speaker, ch) = match chunk.channel {
            Channel::Them => ("them", &mut them),
            Channel::Me => ("me", &mut me),
        };
        let emit = asr.feed(ch, chunk.sample_rate, &chunk.samples);
        match emit {
            Emit::Final(text) => {
                // Same path the Phase-3 player used → the match engine ticks.
                let live = app.state::<LiveState>();
                if let Err(e) = crate::live::feed_transcript_internal(&live, speaker, &text) {
                    tracing::warn!(error = %e, "feed_transcript failed");
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
