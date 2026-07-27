//! Phase-7 verification: does the multilingual Nemotron actually transcribe
//! non-English speech? Runs the model's OWN bundled `test_wavs` (de/es/uk — our
//! launch languages plus German) through the exact shipped `Asr` wrapper and
//! prints each transcript. This is eyes-on proof of the multilingual swap, not
//! a product path — hence an example, never a CI test (the model is 650 MB and
//! dev-resolved).
//!
//! Run:  build-env.bat cargo run --example asr_lang_check --no-default-features
//! (needs sherpa-onnx-c-api.dll + onnxruntime.dll next to the example exe — the
//!  runner script copies them; see verify/asr_lang.ps1.)

use anchor::audio::asr::{model_dir, Asr, Emit};
use std::path::Path;

/// Read a 16-bit or float WAV into mono f32 samples + its sample rate.
fn read_wav(path: &Path) -> (i32, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).expect("open wav");
    let spec = reader.spec();
    let ch = spec.channels.max(1) as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u32 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.expect("sample") as f32 * scale)
                .collect()
        }
        hound::SampleFormat::Float => {
            reader.samples::<f32>().map(|s| s.expect("sample")).collect()
        }
    };
    // Downmix to mono if needed.
    let mono: Vec<f32> = if ch > 1 {
        interleaved
            .chunks(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    } else {
        interleaved
    };
    (spec.sample_rate as i32, mono)
}

/// Feed a whole utterance through one channel and return the transcript.
fn transcribe(asr: &Asr, sr: i32, samples: &[f32], language: &str) -> String {
    let mut ch = asr.new_channel(language);
    let mut finals: Vec<String> = Vec::new();
    let mut pending = String::new();
    // ~200 ms chunks, the cadence the live capture feeds at.
    let chunk = (sr as usize / 5).max(1);
    for w in samples.chunks(chunk) {
        match asr.feed(&mut ch, sr, w) {
            Emit::Final(t) => finals.push(t),
            Emit::Pending(t) => pending = t,
            Emit::Nothing => {}
        }
    }
    // Trailing silence forces the final endpoint on the last segment.
    let silence = vec![0f32; sr as usize];
    if let Emit::Final(t) = asr.feed(&mut ch, sr, &silence) {
        finals.push(t);
    }
    if finals.is_empty() {
        pending
    } else {
        finals.join(" ")
    }
}

fn main() {
    let app_data = Path::new("target/debug/data");
    let dir = model_dir(app_data).expect(
        "multilingual ASR model not found under target/debug/data/models — \
         download it first (see verify/dl)",
    );
    println!("model: {}", dir.display());
    let asr = Asr::load(&dir, 4).expect("load ASR");
    let wavs = dir.join("test_wavs");

    // (file, explicit language code). "auto" exercises the model's own
    // language ID; the ISO code exercises per-stream steering (set_option).
    let cases = [
        ("de.wav", "de"),
        ("de.wav", "auto"),
        ("es.wav", "es"),
        ("uk.wav", "uk"),
    ];
    for (file, lang) in cases {
        let path = wavs.join(file);
        if !path.exists() {
            println!("[{file} lang={lang}] SKIP (missing)");
            continue;
        }
        let (sr, samples) = read_wav(&path);
        let text = transcribe(&asr, sr, &samples, lang);
        let secs = samples.len() as f32 / sr as f32;
        println!("[{file} lang={lang}] ({sr} Hz, {secs:.1}s) -> {text}");
    }
}
