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

    for (channel, direction) in [
        (Channel::Them, Direction::Render), // render endpoint → loopback
        (Channel::Me, Direction::Capture),  // microphone
    ] {
        let stop = stop.clone();
        let tx = tx.clone();
        threads.push(std::thread::spawn(move || {
            if let Err(e) = capture_loop(channel, direction, &stop, &tx) {
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

#[cfg(windows)]
fn capture_loop(
    channel: Channel,
    direction: wasapi::Direction,
    stop: &AtomicBool,
    tx: &Sender<AudioChunk>,
) -> Result<(), String> {
    use std::collections::VecDeque;
    use wasapi::*;

    let _ = initialize_mta();
    let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let device = enumerator
        .get_default_device(&direction)
        .map_err(|e| e.to_string())?;
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

    while !stop.load(Ordering::SeqCst) {
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
                    samples,
                })
                .is_err()
            {
                break; // worker gone
            }
        }
        let _ = h_event.wait_for_event(200);
    }
    client.stop_stream().map_err(|e| e.to_string())?;
    Ok(())
}
