//! Phase-7.5b real-mic pass: capture the actual microphone (the "me" channel,
//! with AEC) and print what the ASR hears, so a human can confirm their own
//! voice transcribes cleanly and the AEC/denoise doesn't chew it up. The "them"
//! (system-audio) line should stay empty while only you speak.
//! Run: build-env.bat cargo run --example mic_check --no-default-features
//! Window: ANCHOR_MIC_SECS (default 50).

use anchor::audio::aec::EchoCanceller;
use anchor::audio::asr::Emit;
use anchor::audio::capture::{start, Channel};
use anchor::audio::engine::{AsrEngine, EnginePref};
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let secs: u64 = std::env::var("ANCHOR_MIC_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    let engine = AsrEngine::load(Path::new("target/debug/data"), 4, EnginePref::Auto).expect("load engine");
    let mut me_ch = engine.new_channel("auto");
    let mut them_ch = engine.new_channel("auto");
    let mut ec = EchoCanceller::new();

    let (tx, rx) = channel();
    let handles = start(tx).expect("capture start");
    println!("engine={} — LISTENING on the microphone for {secs}s. Speak now.", engine.label());

    let (mut me_text, mut them_text) = (Vec::new(), Vec::new());
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(secs) {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
            match chunk.channel {
                Channel::Them => {
                    ec.push_reference(chunk.ts_us, &chunk.samples);
                    if let Emit::Final(t) = engine.feed(&mut them_ch, chunk.sample_rate, &chunk.samples) {
                        them_text.push(t);
                    }
                }
                Channel::Me => {
                    let cleaned = ec.push_mic(chunk.ts_us, &chunk.samples);
                    if !cleaned.is_empty() {
                        if let Emit::Final(t) = engine.feed(&mut me_ch, chunk.sample_rate, &cleaned) {
                            println!("  heard: {t}");
                            me_text.push(t);
                        }
                    }
                }
            }
        }
    }
    handles.stop();
    println!("\n=== ME (your microphone) ===\n{}", me_text.join(" "));
    println!(
        "=== THEM (system audio — should be empty while only you talk) ===\n{}",
        them_text.join(" ")
    );
}
