//! Verify the offline Parakeet fallback + LocalAgreement-2 pseudo-streaming:
//! feed a WAV in 10 ms chunks (the live cadence) through the streaming channel
//! and print the confirmed text. Exercises the real re-decode + agreement +
//! endpoint path, not just a one-shot offline decode.
//! Run: build-env.bat cargo run --example parakeet_check --no-default-features -- <wav>...

use anchor::audio::asr::Emit;
use anchor::audio::asr_offline::{model_dir, ParakeetAsr};
use std::path::Path;

fn read_wav(path: &Path) -> (i32, Vec<f32>) {
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
    (spec.sample_rate as i32, mono)
}

fn main() {
    let dir = model_dir(Path::new("target/debug/data")).expect("parakeet model not found");
    println!("model: {}", dir.display());
    let asr = ParakeetAsr::load(&dir, 4).expect("load parakeet");

    for arg in std::env::args().skip(1) {
        let (lang, path) = match arg.split_once('=') {
            Some((l, p)) => (l.to_string(), p.to_string()),
            None => ("auto".to_string(), arg),
        };
        let (sr, samples) = read_wav(Path::new(&path));
        let mut ch = asr.new_channel(&lang);
        let chunk = (sr / 100).max(1) as usize; // 10 ms, the worker cadence
        let mut finals: Vec<String> = Vec::new();
        let mut last_pending = String::new();
        for w in samples.chunks(chunk) {
            match asr.feed(&mut ch, sr, w) {
                Emit::Final(t) => finals.push(t),
                Emit::Pending(t) => last_pending = t,
                Emit::Nothing => {}
            }
        }
        // 2 s of trailing silence to trip the endpoint and flush the final.
        let sil = vec![0f32; 2 * sr as usize];
        for w in sil.chunks(chunk) {
            if let Emit::Final(t) = asr.feed(&mut ch, sr, w) {
                finals.push(t);
            }
        }
        let text = if finals.is_empty() {
            last_pending
        } else {
            finals.join(" ")
        };
        println!("[{lang}] {path}\n   -> {}\n", text.trim());
    }
}
