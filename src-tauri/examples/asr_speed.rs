//! Phase-7 speed measurement: did swapping the EN-only streaming model for the
//! multilingual one change the CPU budget? Runs the SAME audio through BOTH
//! models on this machine (identical build, identical wavs) and reports model
//! load time, RTF (compute / audio-second), and first-partial compute latency.
//! The multilingual-vs-EN comparison is apples-to-apples; that is the question.
//!
//! Run:  build-env.bat cargo run --example asr_speed --no-default-features
//! (copy the sherpa/onnx DLLs next to the example exe first — verify/asr_lang.ps1
//!  does this; or reuse verify/asr_speed.ps1.)

use anchor::audio::asr::{Asr, Emit};
use std::path::Path;
use std::time::Instant;

const ML: &str = "sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-320ms-int8-2026-06-11";
const EN: &str = "sherpa-onnx-nemotron-speech-streaming-en-0.6b-int8-2026-01-14";

fn read_wav(path: &Path) -> (i32, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).expect("open wav");
    let spec = reader.spec();
    let ch = spec.channels.max(1) as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u32 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.expect("s") as f32 * scale).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.expect("s")).collect(),
    };
    let mono: Vec<f32> = if ch > 1 {
        interleaved.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
    } else {
        interleaved
    };
    (spec.sample_rate as i32, mono)
}

/// Feed the whole utterance as fast as possible, timing the ASR compute.
/// Returns (rtf, first_partial_ms, audio_seconds).
fn measure(asr: &Asr, sr: i32, samples: &[f32]) -> (f64, u128, f64) {
    let mut ch = asr.new_channel("auto");
    let chunk = (sr as usize / 5).max(1); // 200 ms chunks, the live cadence
    let audio_s = samples.len() as f64 / sr as f64;
    let start = Instant::now();
    let mut first_partial: Option<u128> = None;
    for w in samples.chunks(chunk) {
        match asr.feed(&mut ch, sr, w) {
            Emit::Pending(_) | Emit::Final(_) if first_partial.is_none() => {
                first_partial = Some(start.elapsed().as_millis());
            }
            _ => {}
        }
    }
    let compute_ms = start.elapsed().as_millis();
    let rtf = (compute_ms as f64 / 1000.0) / audio_s;
    (rtf, first_partial.unwrap_or(compute_ms), audio_s)
}

fn bench(label: &str, model_dir: &Path, wavs_dir: &Path, files: &[&str]) {
    if !model_dir.join("encoder.int8.onnx").exists() {
        println!("[{label}] SKIP — model not found at {}", model_dir.display());
        return;
    }
    let t = Instant::now();
    let asr = Asr::load(model_dir, 4).expect("load");
    let load_ms = t.elapsed().as_millis();
    println!("\n[{label}] load {load_ms} ms");
    let (mut rtf_sum, mut n) = (0.0, 0.0);
    for &file in files {
        let path = wavs_dir.join(file);
        if !path.exists() {
            continue;
        }
        let (sr, samples) = read_wav(&path);
        // Warm-up run (resampler + graph init) — discarded so the number is steady-state.
        let _ = measure(&asr, sr, &samples);
        let (rtf, first_ms, audio_s) = measure(&asr, sr, &samples);
        println!("  {file:8} {audio_s:.1}s  RTF {rtf:.3}  first-partial {first_ms} ms");
        rtf_sum += rtf;
        n += 1.0;
    }
    if n > 0.0 {
        println!("  mean RTF {:.3}  ({}x realtime)", rtf_sum / n, (n / rtf_sum).round());
    }
}

fn main() {
    let ml_dir = Path::new("target/debug/data/models").join(ML);
    // The EN spike model lives at the repo root, not under src-tauri — try a few
    // parent prefixes so the bench works whatever the cwd.
    let en_dir = ["spike/audio/models", "../spike/audio/models", "../../spike/audio/models"]
        .iter()
        .map(|p| Path::new(p).join(EN))
        .find(|p| p.join("encoder.int8.onnx").exists())
        .unwrap_or_else(|| Path::new("spike/audio/models").join(EN));
    let wavs_dir = ml_dir.join("test_wavs");
    let files = ["de.wav", "es.wav", "uk.wav"];
    println!("Same audio, same machine, both models. RTF = compute / audio-second (lower is faster).");
    bench("multilingual", &ml_dir, &wavs_dir, &files);
    bench("english-only", &en_dir, &wavs_dir, &files);
}
