//! Phase-7.5b / 7.7 soak: run the full hot stack — dual capture + streaming ASR
//! (with AEC) on both channels + the embedder ticking like the live loop — for a
//! configurable duration, printing liveness so RAM can be sampled from outside
//! (Get-Process soak | WorkingSet64) and a long run proves no leak / no crash.
//!
//! Clock drift between the two capture streams is handled by construction now:
//! both are timestamped from the same WASAPI QPC clock (see audio/aec.rs), so
//! their timelines cannot diverge. This harness just proves stability over time.
//!
//! Run:  build-env.bat cargo run --release --example soak --no-default-features
//! Duration: ANCHOR_SOAK_SECS (default 90).

use anchor::audio::aec::EchoCanceller;
use anchor::audio::asr::{model_dir, Asr};
use anchor::audio::capture::{start, Channel};
use anchor::embed::Embedder;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let secs: u64 = std::env::var("ANCHOR_SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);

    let dir = model_dir(Path::new("target/debug/data")).expect("ASR model");
    let asr = Asr::load(&dir, 4).expect("load ASR");
    let embedder = Embedder::new();
    let mut them = asr.new_channel("auto");
    let mut me = asr.new_channel("auto");
    let mut ec = EchoCanceller::new();

    let (tx, rx) = channel();
    let handles = start(tx).expect("capture start");
    println!("soak running for {secs}s — sample RAM with: Get-Process soak | % WorkingSet64");

    let started = Instant::now();
    let mut last_report = Instant::now();
    let mut last_embed = Instant::now();
    let (mut them_chunks, mut me_chunks, mut embeds) = (0u64, 0u64, 0u64);

    while started.elapsed() < Duration::from_secs(secs) {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
            match chunk.channel {
                Channel::Them => {
                    them_chunks += 1;
                    ec.push_reference(chunk.ts_us, &chunk.samples);
                    let _ = asr.feed(&mut them, chunk.sample_rate, &chunk.samples);
                }
                Channel::Me => {
                    me_chunks += 1;
                    let cleaned = ec.push_mic(chunk.ts_us, &chunk.samples);
                    if !cleaned.is_empty() {
                        let _ = asr.feed(&mut me, chunk.sample_rate, &cleaned);
                    }
                }
            }
        }
        // Embed like the live tick (~every 300 ms) to keep the embedder hot.
        if last_embed.elapsed() >= Duration::from_millis(300) {
            let _ = embedder.embed_query("kubernetes migration salary expectations leaving the company");
            embeds += 1;
            last_embed = Instant::now();
        }
        if last_report.elapsed() >= Duration::from_secs(15) {
            println!(
                "t={:>4}s  them={them_chunks}  me={me_chunks}  embeds={embeds}",
                started.elapsed().as_secs()
            );
            last_report = Instant::now();
        }
    }
    handles.stop();
    println!("done: them={them_chunks} me={me_chunks} embeds={embeds}");
}
