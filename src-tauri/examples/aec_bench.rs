//! Phase-7.3 AEC bench — deterministic, no acoustics (the gold-standard way to
//! measure an echo canceller). We synthesize the echo path ourselves so the
//! test is repeatable and free of room variance:
//!   reference = far-end voice (what the loopback "them" captures)
//!   echo      = a delayed, attenuated, multi-tap copy of the far end (the room)
//!   mic       = [user voice +] echo
//! Then we run our real EchoCanceller and measure:
//!   1. ERLE  — echo-return-loss enhancement in dB (echo-only mic): higher = more
//!      echo removed. Speex single-talk typically lands ~15-30 dB.
//!   2. double-talk — transcribe the cancelled output when the user talks OVER
//!      the echo; it must read as the USER, not the far end, and the user's
//!      energy must be preserved.
//!   3. residual vs speech RMS — to set the output energy gate from data.
//!
//! Run:  build-env.bat cargo run --example aec_bench --no-default-features

use anchor::audio::aec::EchoCanceller;
use anchor::audio::asr::{model_dir, Asr, Emit};
use std::path::Path;

const FRAME: usize = 160; // 10 ms @ 16 kHz

fn read_16k_mono(path: &Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(path).expect("open wav");
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let inter: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let s = 1.0 / (1u32 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|x| x.unwrap() as f32 * s).collect()
        }
        hound::SampleFormat::Float => r.samples::<f32>().map(|x| x.unwrap()).collect(),
    };
    let mono: Vec<f32> = if ch > 1 {
        inter.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
    } else {
        inter
    };
    resample(&mono, spec.sample_rate, 16_000)
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let i0 = src.floor() as usize;
            let frac = (src - i0 as f64) as f32;
            let a = input.get(i0).copied().unwrap_or(0.0);
            let b = input.get(i0 + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// Build a room echo: delayed + attenuated, two taps (direct + one reflection).
fn make_echo(farend: &[f32]) -> Vec<f32> {
    let d1 = (0.045 * 16_000.0) as usize; // 45 ms
    let d2 = (0.075 * 16_000.0) as usize; // 75 ms reflection
    let (g1, g2) = (0.45f32, 0.2f32);
    let mut echo = vec![0f32; farend.len()];
    for n in 0..farend.len() {
        let mut v = 0.0;
        if n >= d1 {
            v += g1 * farend[n - d1];
        }
        if n >= d2 {
            v += g2 * farend[n - d2];
        }
        echo[n] = v;
    }
    echo
}

/// Run mic+reference through the canceller and return the cancelled output,
/// aligned to sample 0 of the mic.
fn cancel(reference: &[f32], mic: &[f32], preprocess: bool) -> Vec<f32> {
    let mut ec = EchoCanceller::with_preprocess(preprocess);
    let mut out = Vec::with_capacity(mic.len());
    let n = reference.len().max(mic.len());
    for (k, start) in (0..n).step_by(FRAME).enumerate() {
        let ts = (k as u64) * 10_000; // first-sample time of this 10 ms frame
        let end = (start + FRAME).min(n);
        let mut rf = vec![0f32; FRAME];
        let mut mf = vec![0f32; FRAME];
        for i in start..end {
            rf[i - start] = reference.get(i).copied().unwrap_or(0.0);
            mf[i - start] = mic.get(i).copied().unwrap_or(0.0);
        }
        ec.push_reference(ts, &rf);
        out.extend(ec.push_mic(ts, &mf));
    }
    out
}

fn transcribe(asr: &Asr, samples: &[f32]) -> String {
    let mut ch = asr.new_channel("auto");
    let mut text = String::new();
    for w in samples.chunks(FRAME) {
        if let Emit::Final(t) = asr.feed(&mut ch, 16_000, w) {
            text.push_str(&t);
            text.push(' ');
        }
    }
    // 3 s of trailing silence — enough to trip the endpoint (rule1 = 2.4 s) so a
    // short clip's final segment is actually emitted.
    let sil = vec![0f32; 3 * 16_000];
    if let Emit::Final(t) = asr.feed(&mut ch, 16_000, &sil) {
        text.push_str(&t);
        text.push(' ');
    }
    text.trim().to_string()
}

fn main() {
    let farend = read_16k_mono(Path::new("../spike/voices/dialogue.wav"));
    let echo = make_echo(&farend);
    let mic_echo = echo.clone();

    let dt_user = read_16k_mono(Path::new("../spike/voices/2_kubernetes.wav")); // "kubernetes migration"
    let dt_far = read_16k_mono(Path::new("../spike/voices/1_leaving.wav")); // "leaving your own company"
    let dt_echo = make_echo(&dt_far);
    let dtlen = dt_user.len().min(dt_echo.len());
    let mic_dt: Vec<f32> = (0..dtlen).map(|n| dt_user[n] + dt_echo[n]).collect();

    let asr = Asr::load(&model_dir(Path::new("target/debug/data")).expect("model"), 4).expect("asr");
    println!("USER alone   : {}", transcribe(&asr, &dt_user[..dtlen]));
    println!("FAR-END alone: {}", transcribe(&asr, &dt_far));
    println!("MIC raw (dt) : {}\n", transcribe(&asr, &mic_dt));

    for &preprocess in &[true, false] {
        println!("################ preprocess = {preprocess} ################");

        // 1. ERLE — echo-only mic (single-talk), steady state (skip 1 s warmup).
        let out_echo = cancel(&farend, &mic_echo, preprocess);
        let skip = 16_000.min(out_echo.len());
        let mic_ss = &mic_echo[skip..out_echo.len().max(skip)];
        let out_ss = &out_echo[skip..];
        let e_in = mic_ss.iter().map(|v| v * v).sum::<f32>();
        let e_out = out_ss.iter().map(|v| v * v).sum::<f32>().max(1e-9);
        let erle = 10.0 * (e_in / e_out).log10();
        println!(
            "  single-talk: echo RMS {:.4} -> residual {:.4}   ERLE {erle:.1} dB",
            rms(mic_ss),
            rms(out_ss)
        );

        // 2. Double-talk — user over a different far-end sentence.
        let out_dt = cancel(&dt_far[..dtlen.min(dt_far.len())], &mic_dt, preprocess);
        println!(
            "  double-talk: user RMS {:.4} -> output RMS {:.4}",
            rms(&dt_user[..dtlen]),
            rms(&out_dt)
        );
        println!("  double-talk OUTPUT (should read as USER): {}", transcribe(&asr, &out_dt));
    }
}
