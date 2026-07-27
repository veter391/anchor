//! Dual-channel capture on Windows, both via the `wasapi` crate:
//! - "them" = the default render endpoint opened for Capture ⇒ WASAPI loopback
//! - "me"   = the default capture endpoint (microphone), normal capture
//!
//! One audio crate, one windows version (cpal 0.18 clashes with Tauri's
//! windows stack). Both request 16 kHz mono f32 with autoconvert, so no
//! resampling or downmix is needed here. Chunks go to the ASR worker over a
//! channel. No packets arrive during silence — the event wait times out and
//! we loop; the ASR's endpoint rule owns "trailing silence".

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

pub const RATE: i32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Them,
    Me,
}

pub struct AudioChunk {
    pub channel: Channel,
    pub sample_rate: i32,
    pub samples: Vec<f32>,
    /// Microseconds since a shared capture origin, stamped when the chunk was
    /// drained. Both channels share the origin, so their timestamps sit on one
    /// timeline — the AEC reference aligner (audio/aec.rs) needs this to place
    /// the intermittent "them" loopback against the continuous mic.
    pub ts_us: u64,
}

pub struct CaptureHandles {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl CaptureHandles {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
    }
}

#[cfg(windows)]
pub fn start(tx: Sender<AudioChunk>) -> Result<CaptureHandles, String> {
    use wasapi::Direction;
    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();
    // One origin shared by both threads → their chunk timestamps share a timeline.
    let origin = Instant::now();

    for (channel, direction) in [
        (Channel::Them, Direction::Render), // render endpoint → loopback
        (Channel::Me, Direction::Capture),  // microphone
    ] {
        let stop = stop.clone();
        let tx = tx.clone();
        threads.push(std::thread::spawn(move || {
            if let Err(e) = capture_loop(channel, direction, &stop, &tx, origin) {
                tracing::warn!(?channel, error = %e, "capture thread stopped");
            }
        }));
    }

    Ok(CaptureHandles { stop, threads })
}

#[cfg(not(windows))]
pub fn start(_tx: Sender<AudioChunk>) -> Result<CaptureHandles, String> {
    Err("audio capture is Windows-only in this build".into())
}

/// Why a single stream run ended — so the supervisor knows whether to rebuild.
#[cfg(windows)]
enum StreamExit {
    /// The app asked capture to stop (or the ASR worker went away): do not rebuild.
    Stopped,
    /// The default endpoint changed under us (Bluetooth A2DP↔HFP switch, unplug,
    /// user-picked a new device): rebuild on the new default.
    DeviceChanged,
}

/// Supervises one channel's capture: opens the default endpoint and, when it
/// changes mid-call or the stream errors, rebuilds in place instead of dying.
///
/// The `wasapi` crate does not wrap `IMMNotificationClient` (enumerator-level
/// default-device-changed), so we detect the change by polling the current
/// default endpoint id (~1 s) and comparing it to the one we opened. A silent
/// Bluetooth profile switch leaves the old stream error-free but no longer the
/// default, so polling — not a read error — is what catches it; read errors
/// (device removal) are handled too. Acceptance bar (02 §1 / 09 Phase 7):
/// recover automatically, never fail silently. The dead-channel watchdog
/// (audio/mod.rs) surfaces the brief gap to the UI.
#[cfg(windows)]
fn capture_loop(
    channel: Channel,
    direction: wasapi::Direction,
    stop: &AtomicBool,
    tx: &Sender<AudioChunk>,
    origin: Instant,
) -> Result<(), String> {
    use wasapi::initialize_mta;

    let _ = initialize_mta();
    while !stop.load(Ordering::SeqCst) {
        match run_stream_once(channel, &direction, stop, tx, origin) {
            Ok(StreamExit::Stopped) => break,
            Ok(StreamExit::DeviceChanged) => {
                tracing::info!(?channel, "audio default device changed — rebuilding capture");
            }
            Err(e) => {
                tracing::warn!(?channel, error = %e, "capture stream error — rebuilding");
                // Back off so a persistently-failing device doesn't hot-loop.
                for _ in 0..10 {
                    if stop.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    Ok(())
}

/// One capture stream, from open to teardown. Returns how it ended.
#[cfg(windows)]
fn run_stream_once(
    channel: Channel,
    direction: &wasapi::Direction,
    stop: &AtomicBool,
    tx: &Sender<AudioChunk>,
    origin: Instant,
) -> Result<StreamExit, String> {
    use std::collections::VecDeque;
    use wasapi::*;

    let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let device = enumerator
        .get_default_device(direction)
        .map_err(|e| e.to_string())?;
    let opened_id = device.get_id().map_err(|e| e.to_string())?;
    let mut client = device.get_iaudioclient().map_err(|e| e.to_string())?;

    let desired = WaveFormat::new(32, 32, &SampleType::Float, RATE as usize, 1, None);
    let (_def, min_time) = client.get_device_period().map_err(|e| e.to_string())?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_time,
    };
    // A render device initialized for Capture engages loopback; a capture
    // device stays a normal microphone capture.
    client
        .initialize_client(&desired, &Direction::Capture, &mode)
        .map_err(|e| e.to_string())?;
    let h_event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    let mut queue: VecDeque<u8> = VecDeque::new();
    client.start_stream().map_err(|e| e.to_string())?;

    // Poll the default endpoint every ~1 s (5 × the 200 ms event timeout).
    let mut ticks: u32 = 0;
    let exit = loop {
        if stop.load(Ordering::SeqCst) {
            break StreamExit::Stopped;
        }
        capture
            .read_from_device_to_deque(&mut queue)
            .map_err(|e| e.to_string())?;
        if queue.len() >= 4 {
            let mut samples = Vec::with_capacity(queue.len() / 4);
            let mut buf = [0u8; 4];
            while queue.len() >= 4 {
                for b in &mut buf {
                    *b = queue.pop_front().unwrap();
                }
                samples.push(f32::from_le_bytes(buf));
            }
            if tx
                .send(AudioChunk {
                    channel,
                    sample_rate: RATE,
                    ts_us: origin.elapsed().as_micros() as u64,
                    samples,
                })
                .is_err()
            {
                break StreamExit::Stopped; // worker gone — nothing to recover
            }
        }
        ticks += 1;
        if ticks >= 5 {
            ticks = 0;
            if default_device_changed(&enumerator, direction, &opened_id) {
                break StreamExit::DeviceChanged;
            }
        }
        let _ = h_event.wait_for_event(200);
    };
    let _ = client.stop_stream();
    Ok(exit)
}

/// True iff the current default endpoint id differs from the one we opened.
/// A transient enumeration failure is treated as "unchanged" — we do not tear
/// down a working stream over a momentary query hiccup.
#[cfg(windows)]
fn default_device_changed(
    enumerator: &wasapi::DeviceEnumerator,
    direction: &wasapi::Direction,
    opened_id: &str,
) -> bool {
    match enumerator
        .get_default_device(direction)
        .and_then(|d| d.get_id())
    {
        Ok(current) => current != opened_id,
        Err(_) => false,
    }
}
