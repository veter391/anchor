//! Verify the ASR engine abstraction end to end via the exact path the worker
//! uses: AsrEngine::load (selection) → new_channel → feed (enum dispatch). Pick
//! the engine with ANCHOR_ASR_ENGINE=streaming|offline|auto.
//! Run: build-env.bat cargo run --example engine_check --no-default-features -- <wav>...

use anchor::audio::asr::Emit;
use anchor::audio::engine::{AsrEngine, EnginePref};
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
    let pref = EnginePref::parse(&std::env::var("ANCHOR_ASR_ENGINE").unwrap_or_default());
    let engine = AsrEngine::load(Path::new("target/debug/data"), 4, pref).expect("load engine");
    println!("pref={pref:?}  engine={}", engine.label());

    for arg in std::env::args().skip(1) {
        let (lang, path) = match arg.split_once('=') {
            Some((l, p)) => (l.to_string(), p.to_string()),
            None => ("auto".to_string(), arg),
        };
        let (sr, samples) = read_wav(Path::new(&path));
        let mut ch = engine.new_channel(&lang);
        let chunk = (sr / 100).max(1) as usize; // 10 ms, the worker cadence
        let mut finals: Vec<String> = Vec::new();
        let mut pending = String::new();
        for w in samples.chunks(chunk) {
            match engine.feed(&mut ch, sr, w) {
                Emit::Final(t) => finals.push(t),
                Emit::Pending(t) => pending = t,
                Emit::Nothing => {}
            }
        }
        let sil = vec![0f32; 2 * sr as usize];
        for w in sil.chunks(chunk) {
            if let Emit::Final(t) = engine.feed(&mut ch, sr, w) {
                finals.push(t);
            }
        }
        let text = if finals.is_empty() { pending } else { finals.join(" ") };
        println!("[{lang}] {path}\n   -> {}\n", text.trim());
    }
}
