//! Phase-7 verification: does dual capture survive a default-device change
//! mid-run instead of dying? Starts the real capture path, prints per-second
//! chunk deltas per channel, and (via tracing) logs each stream rebuild. Switch
//! the default PLAYBACK device while this runs — the "them" loopback stream
//! should log a rebuild and keep going, and the "me" mic stream should never
//! stall. Run through verify/capture_probe.ps1 (builds, copies DLLs, flips the
//! default device on a timer). Not a product path — a device-resilience probe.

use anchor::audio::capture::{start, Channel};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .init();

    let (tx, rx) = channel();
    let handles = start(tx).expect("capture start");
    let started = Instant::now();
    let run = Duration::from_secs(30);

    let (mut them, mut me) = (0u64, 0u64);
    let (mut prev_them, mut prev_me) = (0u64, 0u64);
    let mut last_print = Instant::now();

    while started.elapsed() < run {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
            match chunk.channel {
                Channel::Them => them += 1,
                Channel::Me => me += 1,
            }
        }
        if last_print.elapsed() >= Duration::from_secs(1) {
            let s = started.elapsed().as_secs();
            println!(
                "t={s:>2}s  them +{:<4} (tot {them})   me +{:<4} (tot {me})",
                them - prev_them,
                me - prev_me
            );
            prev_them = them;
            prev_me = me;
            last_print = Instant::now();
        }
    }
    handles.stop();
    println!("done — them={them} me={me} (me must stay > 0 throughout = capture never died)");
}
