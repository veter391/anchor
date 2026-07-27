//! Signal-level acoustic echo cancellation for the open-speakers case, with
//! time-alignment — the part a naive integration gets wrong.
//!
//! Without headphones the mic hears the other side out loud from the speakers,
//! so the far-end voice lands on BOTH channels — "them" (loopback) and "me"
//! (mic). We already hold the far end as the loopback "them" stream, so we use
//! it as the reference and subtract it from the mic with speexdsp (`aec-rs`).
//!
//! The catch (measured, 10_RESEARCH_LOG 2026-07-27): the loopback emits samples
//! ONLY while the far end is talking — it has gaps during far-end silence. speex
//! needs the reference as a CONTINUOUS real-time signal aligned to the mic; feed
//! it a gap-compressed reference and it cancels nothing. So both streams carry a
//! shared-origin capture timestamp, and we reconstruct each on that common
//! timeline (`TimeLine`), zero-filling the reference's silence gaps, before
//! cancelling frame by frame. With headphones there is no acoustic echo, the mic
//! doesn't correlate with the reference, and speex is a near-passthrough — so
//! this is safe to run always.

use aec_rs::{Aec, AecConfig};
use std::collections::VecDeque;

const RATE: i64 = 16_000;
/// speex frame = 10 ms at 16 kHz — `cancel_echo` works one frame at a time.
const FRAME: usize = 160;
/// Cancel the mic this far behind its newest sample, so a slightly-late "them"
/// chunk for the same instant has arrived before we process that frame (~100 ms).
const LAG: i64 = 1_600;
/// Never zero-fill a gap larger than this (timestamp glitch / very long silence):
/// reset the line instead, to bound memory and avoid a pathological fill.
const MAX_GAP: i64 = RATE;

/// A sample stream reconstructed on the shared capture timeline: contiguous
/// samples with `start` = the timeline sample-position of `data[0]`. Reference
/// silence gaps are zero-filled so the line stays a continuous real-time signal.
struct TimeLine {
    data: VecDeque<i16>,
    start: i64,
    end: i64,
}

impl TimeLine {
    fn new() -> Self {
        Self {
            data: VecDeque::new(),
            start: 0,
            end: 0,
        }
    }

    /// Write a chunk whose first sample sits at `pos` on the timeline.
    fn write(&mut self, pos: i64, samples: &[i16]) {
        if self.data.is_empty() {
            self.start = pos;
            self.end = pos;
        }
        if pos > self.end {
            let gap = pos - self.end;
            if gap > MAX_GAP {
                self.data.clear();
                self.start = pos;
                self.end = pos;
            } else {
                self.data.resize(self.data.len() + gap as usize, 0);
                self.end = pos;
            }
        }
        // pos < end (out-of-order / overlap) just appends contiguously: we drop
        // the backward timing rather than rewrite history — rare, speex tolerates
        // a frame of jitter.
        self.data.extend(samples.iter().copied());
        self.end += samples.len() as i64;
    }

    /// Fill `out` with FRAME samples starting at `pos`; positions outside the
    /// buffered range are silence (0).
    fn read_frame(&self, pos: i64, out: &mut [i16; FRAME]) {
        for (i, o) in out.iter_mut().enumerate() {
            let p = pos + i as i64;
            *o = if p >= self.start && p < self.end {
                self.data[(p - self.start) as usize]
            } else {
                0
            };
        }
    }

    /// Drop everything before `pos` (already consumed).
    fn trim_before(&mut self, pos: i64) {
        while self.start < pos && !self.data.is_empty() {
            self.data.pop_front();
            self.start += 1;
        }
        if self.data.is_empty() {
            self.start = pos;
            self.end = self.end.max(pos);
        }
    }
}

pub struct EchoCanceller {
    aec: Aec,
    reference: TimeLine,
    mic: TimeLine,
    next_pos: Option<i64>,
    newest_mic: i64,
}

impl EchoCanceller {
    pub fn new() -> Self {
        // Defaults: frame 160, filter 1600 (100 ms tail), 16 kHz, preprocess on
        // (residual-echo suppression + light denoise — good for ASR).
        Self {
            aec: Aec::new(&AecConfig::default()),
            reference: TimeLine::new(),
            mic: TimeLine::new(),
            next_pos: None,
            newest_mic: 0,
        }
    }

    /// Timeline sample-position of a chunk's first sample: samples elapsed at
    /// read time, minus the chunk length.
    fn pos_of(ts_us: u64, len: usize) -> i64 {
        (ts_us as i64 * RATE) / 1_000_000 - len as i64
    }

    /// Buffer far-end ("them" loopback) samples as the echo reference.
    pub fn push_reference(&mut self, ts_us: u64, samples: &[f32]) {
        let pos = Self::pos_of(ts_us, samples.len());
        let s: Vec<i16> = samples.iter().map(|&x| to_i16(x)).collect();
        self.reference.write(pos, &s);
    }

    /// Feed mic ("me") samples; return echo-cancelled samples ready for ASR.
    /// Output lags real time by ~LAG so the reference for each frame has settled.
    pub fn push_mic(&mut self, ts_us: u64, samples: &[f32]) -> Vec<f32> {
        let pos = Self::pos_of(ts_us, samples.len());
        let s: Vec<i16> = samples.iter().map(|&x| to_i16(x)).collect();
        self.mic.write(pos, &s);
        self.newest_mic = self.mic.end;
        if self.next_pos.is_none() {
            self.next_pos = Some(self.mic.start);
        }
        self.drain()
    }

    fn drain(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        let mut rec = [0i16; FRAME];
        let mut echo = [0i16; FRAME];
        let mut clean = [0i16; FRAME];
        while let Some(np) = self.next_pos {
            if self.newest_mic < np + FRAME as i64 + LAG {
                break; // wait until the reference for this frame has settled
            }
            self.mic.read_frame(np, &mut rec);
            self.reference.read_frame(np, &mut echo);
            self.aec.cancel_echo(&rec, &echo, &mut clean);
            out.extend(clean.iter().map(|&c| to_f32(c)));
            let np2 = np + FRAME as i64;
            self.next_pos = Some(np2);
            self.mic.trim_before(np2);
            self.reference.trim_before(np2);
        }
        out
    }
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0) as i16
}

#[inline]
fn to_f32(s: i16) -> f32 {
    s as f32 / 32768.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_zero_fills_gaps_and_reads_aligned() {
        let mut tl = TimeLine::new();
        tl.write(0, &[5i16; FRAME]); // [0, 160) = 5
        tl.write(2 * FRAME as i64, &[9i16; FRAME]); // gap [160,320)=0, [320,480)=9
        let mut f = [0i16; FRAME];
        tl.read_frame(0, &mut f);
        assert!(f.iter().all(|&x| x == 5), "first frame is the written value");
        tl.read_frame(FRAME as i64, &mut f);
        assert!(f.iter().all(|&x| x == 0), "the silence gap reads as zeros");
        tl.read_frame(2 * FRAME as i64, &mut f);
        assert!(f.iter().all(|&x| x == 9), "post-gap frame aligns to its position");
    }

    #[test]
    fn timeline_reads_before_and_after_range_as_silence() {
        let mut tl = TimeLine::new();
        tl.write(FRAME as i64, &[7i16; FRAME]); // only [160, 320)
        let mut f = [0i16; FRAME];
        tl.read_frame(0, &mut f); // entirely before
        assert!(f.iter().all(|&x| x == 0));
        tl.read_frame(2 * FRAME as i64, &mut f); // entirely after
        assert!(f.iter().all(|&x| x == 0));
    }

    #[test]
    fn pos_of_places_a_10ms_chunk_read_at_10ms_at_origin() {
        // 160 samples = 10 ms; read at t=10 ms → occupies [0, 160).
        assert_eq!(EchoCanceller::pos_of(10_000, FRAME), 0);
        // A second 10 ms chunk read at t=20 ms → [160, 320).
        assert_eq!(EchoCanceller::pos_of(20_000, FRAME), FRAME as i64);
    }

    #[test]
    fn canceller_emits_lagged_frames_and_survives_a_reference_gap() {
        let mut ec = EchoCanceller::new();
        // Feed 40 × 10 ms mic chunks (400 ms) with matching timestamps; feed a
        // reference only for the middle stretch (a gap on both sides).
        let mut total = 0usize;
        for k in 0..40u64 {
            let ts = (k + 1) * 10_000; // 10 ms cadence
            if (10..20).contains(&k) {
                ec.push_reference(ts, &[0.2f32; FRAME]);
            }
            total += ec.push_mic(ts, &[0.1f32; FRAME]).len();
        }
        // With a 100 ms lag we should have emitted most frames, quantised to FRAME.
        assert!(total >= 25 * FRAME, "emitted the bulk of the frames: {total}");
        assert_eq!(total % FRAME, 0, "output is frame-quantised");
    }
}
