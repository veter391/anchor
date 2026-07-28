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
/// chunk for the same instant has arrived before we process that frame. With
/// sample-accurate QPC timestamps the clocks don't skew, so this only needs to
/// cover inter-thread delivery jitter (~50 ms).
const LAG: i64 = 800;
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
                // Long discontinuity (capture rebuild / long pause): drop history
                // and re-anchor the timeline at the new QPC position.
                self.data.clear();
                self.start = pos;
                self.end = pos;
            } else {
                // Small forward gap: zero-fill so positions stay QPC-accurate.
                self.data.resize(self.data.len() + gap as usize, 0);
                self.end = pos;
            }
        }
        // Now pos <= end. When pos < end the chunk overlaps samples we already
        // hold — the device sample-clock ran slightly ahead of the QPC timeline.
        // Appending it whole would push `end` permanently past true QPC time: a
        // one-directional drift that, over a long call, slides the reference out
        // of speex's 100 ms filter window and silently kills cancellation. Keep
        // `end` locked to QPC — append only the part beyond `end`, drop the rest.
        let skip = (self.end - pos).clamp(0, samples.len() as i64) as usize;
        self.data.extend(samples[skip..].iter().copied());
        self.end += (samples.len() - skip) as i64;
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
        // preprocess OFF, by measurement (aec_bench, 10_RESEARCH_LOG): ON gives
        // 40 dB single-talk cancellation but suppresses the near-end ~10 dB in
        // double-talk (the user's own words become untranscribable); OFF keeps a
        // strong 22 dB and preserves the user (−5 dB), which the coverage path
        // needs. Residual echo is handled by the text-level filter.
        Self::with_preprocess(false)
    }

    /// `preprocess` toggles speex's post-filter (residual-echo suppression +
    /// denoise). It sharpens single-talk cancellation but over-suppresses the
    /// near-end during double-talk — the bench measures the trade.
    pub fn with_preprocess(preprocess: bool) -> Self {
        let config = AecConfig {
            enable_preprocess: preprocess,
            ..AecConfig::default() // frame 160, filter 1600 (100 ms), 16 kHz
        };
        Self {
            aec: Aec::new(&config),
            reference: TimeLine::new(),
            mic: TimeLine::new(),
            next_pos: None,
            newest_mic: 0,
        }
    }

    /// Timeline sample-position of a chunk's FIRST sample, from its QPC time.
    fn pos_of(ts_us: u64) -> i64 {
        (ts_us as i64 * RATE) / 1_000_000
    }

    /// Buffer far-end ("them" loopback) samples as the echo reference.
    pub fn push_reference(&mut self, ts_us: u64, samples: &[f32]) {
        let pos = Self::pos_of(ts_us);
        let s: Vec<i16> = samples.iter().map(|&x| to_i16(x)).collect();
        self.reference.write(pos, &s);
    }

    /// Feed mic ("me") samples; return echo-cancelled samples ready for ASR.
    /// Output lags real time by ~LAG so the reference for each frame has settled.
    pub fn push_mic(&mut self, ts_us: u64, samples: &[f32]) -> Vec<f32> {
        let pos = Self::pos_of(ts_us);
        let s: Vec<i16> = samples.iter().map(|&x| to_i16(x)).collect();
        self.mic.write(pos, &s);
        self.newest_mic = self.mic.end;
        match self.next_pos {
            None => self.next_pos = Some(self.mic.start),
            // A mic-timeline reset (capture rebuild / long gap) jumps `start` far
            // ahead. Without moving `next_pos` up, drain would grind out a burst
            // of zero frames from the stale position to the new audio (a false
            // silence stretch into the ASR). Skip straight to the new start.
            Some(np) if np < self.mic.start => self.next_pos = Some(self.mic.start),
            Some(_) => {}
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
    fn overlap_keeps_end_locked_to_qpc() {
        // A fast device clock delivers a chunk whose QPC position (pos) lags the
        // samples already written (pos < end). `end` must stay at the QPC-derived
        // position and NOT accumulate the overlap, or the mic timeline drifts
        // permanently ahead of the reference and defeats cancellation.
        let mut tl = TimeLine::new();
        tl.write(0, &[1i16; FRAME]); // end = 160
        assert_eq!(tl.end, FRAME as i64);
        // Claims pos 120 (overlaps the last 40 samples) but carries a full frame.
        tl.write(120, &[2i16; FRAME]);
        // Only the 120-onward tail is new: end = 120 + 160 = 280, not 160 + 160.
        assert_eq!(tl.end, 120 + FRAME as i64);
        // A wholly-covered chunk (skip >= len) adds nothing and never moves end.
        let before = tl.end;
        tl.write(0, &[3i16; FRAME]);
        assert_eq!(tl.end, before);
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
    fn pos_of_maps_first_sample_time_to_sample_index() {
        // ts is the FIRST sample's time: t=0 → sample 0, t=10 ms → sample 160.
        assert_eq!(EchoCanceller::pos_of(0), 0);
        assert_eq!(EchoCanceller::pos_of(10_000), FRAME as i64);
    }

    #[test]
    fn canceller_emits_lagged_frames_and_survives_a_reference_gap() {
        let mut ec = EchoCanceller::new();
        // Feed 60 × 10 ms mic chunks (600 ms) with first-sample timestamps; feed
        // a reference only for the middle stretch (a gap on both sides).
        let mut total = 0usize;
        for k in 0..60u64 {
            let ts = k * 10_000; // first sample of chunk k at k×10 ms
            if (20..40).contains(&k) {
                ec.push_reference(ts, &[0.2f32; FRAME]);
            }
            total += ec.push_mic(ts, &[0.1f32; FRAME]).len();
        }
        // With the ~50 ms lag we emit the bulk of the frames, quantised to FRAME.
        assert!(total >= 45 * FRAME, "emitted the bulk of the frames: {total}");
        assert_eq!(total % FRAME, 0, "output is frame-quantised");
    }
}
