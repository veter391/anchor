//! Phase-7.3 AEC effectiveness A/B: plays a REAL recorded voice (our HiggsField
//! seed voices in spike/voices, not synthesized TTS) out of the default speakers
//! (the "them" side) and prints what each channel transcribes, driving the SAME
//! time-aligned EchoCanceller the worker uses. Run twice:
//!   AEC on  (default):             me should NOT carry the phrase (echo cancelled)
//!   AEC off (ANCHOR_DISABLE_AEC=1): me SHOULD carry the phrase (raw mic echo)
//! "them" (digital loopback) carries the phrase either way. If "me" is empty in
//! BOTH runs the speakers were inaudible to the mic — inconclusive, not proof.
//!
//! Run:  build-env.bat cargo run --example aec_verify --no-default-features
//! WAV:  ANCHOR_AEC_WAV=../spike/voices/dialogue.wav ANCHOR_AEC_SECS=20

use anchor::audio::aec::EchoCanceller;
use anchor::audio::asr::{model_dir, Asr, Emit};
use anchor::audio::capture::{start, Channel};
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let dir = model_dir(Path::new("target/debug/data")).expect("ASR model");
    let asr = Asr::load(&dir, 4).expect("load ASR");
    let mut them = asr.new_channel("auto");
    let mut me = asr.new_channel("auto");

    let wav = std::env::var("ANCHOR_AEC_WAV")
        .unwrap_or_else(|_| "../spike/voices/1_leaving.wav".to_string());
    let secs: u64 = std::env::var("ANCHOR_AEC_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);
    let aec_on = std::env::var_os("ANCHOR_DISABLE_AEC").is_none();
    // Diagnostic: run AEC on the mic but never feed it a reference (empty = all
    // zeros). Simulates the headphones/no-echo case — measures whether the
    // speex preprocess alone degrades clean speech reaching the mic.
    let no_ref = std::env::var_os("ANCHOR_AEC_NOREF").is_some();
    let mut ec = EchoCanceller::new();

    let (tx, rx) = channel();
    let handles = start(tx).expect("capture start");

    let wav_play = wav.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        let script = format!("(New-Object Media.SoundPlayer '{wav_play}').PlaySync()");
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status();
    });

    let (mut them_txt, mut me_txt) = (String::new(), String::new());
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(secs) {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
            match chunk.channel {
                Channel::Them => {
                    if !no_ref {
                        ec.push_reference(chunk.ts_us, &chunk.samples);
                    }
                    if let Emit::Final(t) = asr.feed(&mut them, chunk.sample_rate, &chunk.samples) {
                        them_txt.push_str(&t);
                        them_txt.push(' ');
                    }
                }
                Channel::Me => {
                    let cleaned;
                    let samples: &[f32] = if aec_on {
                        cleaned = ec.push_mic(chunk.ts_us, &chunk.samples);
                        &cleaned
                    } else {
                        &chunk.samples
                    };
                    if !samples.is_empty() {
                        if let Emit::Final(t) = asr.feed(&mut me, chunk.sample_rate, samples) {
                            me_txt.push_str(&t);
                            me_txt.push(' ');
                        }
                    }
                }
            }
        }
    }
    handles.stop();

    let aec = if aec_on { "ON (speexdsp, aligned)" } else { "OFF (raw mic)" };
    println!("\n=== AEC {aec} ===");
    println!("voice played  : {wav}");
    println!("THEM (loopback): {}", them_txt.trim());
    println!("ME   (mic)     : {}", me_txt.trim());
}
