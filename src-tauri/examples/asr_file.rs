//! Transcribe arbitrary WAV files through the shipped multilingual ASR. Used to
//! verify generated ES/RU/UK voices before the cross-lingual retrieval test.
//! Run:  build-env.bat cargo run --example asr_file --no-default-features -- <wav>...
//! Optionally force a language: prefix a path with "de=" / "ru=" etc.

use anchor::audio::asr::{model_dir, Asr, Emit};
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
    let dir = model_dir(Path::new("target/debug/data")).expect("ASR model");
    let asr = Asr::load(&dir, 4).expect("load ASR");
    for arg in std::env::args().skip(1) {
        let (lang, path) = match arg.split_once('=') {
            Some((l, p)) => (l.to_string(), p.to_string()),
            None => ("auto".to_string(), arg),
        };
        let (sr, samples) = read_wav(Path::new(&path));
        let mut ch = asr.new_channel(&lang);
        let mut text = String::new();
        for w in samples.chunks((sr as usize / 5).max(1)) {
            if let Emit::Final(t) | Emit::Pending(t) = asr.feed(&mut ch, sr, w) {
                text = t;
            }
        }
        let sil = vec![0f32; 3 * sr as usize];
        if let Emit::Final(t) = asr.feed(&mut ch, sr, &sil) {
            text = t;
        }
        println!("[{lang}] {path}\n   -> {}\n", text.trim());
    }
}
