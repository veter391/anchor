//! Streaming ASR: one Nemotron recognizer, one stream per channel. Emits a
//! growing `pending` partial and, at each endpoint (trailing silence), a
//! `final` segment. The final segments feed the match engine at phrase
//! cadence — the exact cadence the Phase-3 transcript player already proved.
//!
//! Model files are resolved at runtime (never bundled — 633 MB, and the
//! Nemotron licence is redistribution-noticed); see `model_dir`.

use sherpa_onnx::{
    OnlineModelConfig, OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
    OnlineTransducerModelConfig,
};
use std::path::{Path, PathBuf};

pub struct Asr {
    recognizer: OnlineRecognizer,
}

pub struct ChannelStream {
    stream: OnlineStream,
    last_pending: String,
}

#[derive(Debug, Clone)]
pub enum Emit {
    /// Growing partial for the current utterance (live UI only).
    Pending(String),
    /// A completed utterance — feeds the match engine and the transcript.
    Final(String),
    Nothing,
}

impl Asr {
    pub fn load(model_dir: &Path, num_threads: i32) -> Result<Self, String> {
        let enc = model_dir.join("encoder.int8.onnx");
        let dec = model_dir.join("decoder.int8.onnx");
        let joi = model_dir.join("joiner.int8.onnx");
        let tok = model_dir.join("tokens.txt");
        for p in [&enc, &dec, &joi, &tok] {
            if !p.exists() {
                return Err(format!("missing ASR model file: {}", p.display()));
            }
        }
        let config = OnlineRecognizerConfig {
            model_config: OnlineModelConfig {
                transducer: OnlineTransducerModelConfig {
                    encoder: Some(enc.to_string_lossy().into_owned()),
                    decoder: Some(dec.to_string_lossy().into_owned()),
                    joiner: Some(joi.to_string_lossy().into_owned()),
                },
                tokens: Some(tok.to_string_lossy().into_owned()),
                num_threads,
                provider: Some("cpu".to_string()),
                debug: false,
                ..Default::default()
            },
            decoding_method: Some("greedy_search".to_string()),
            // Endpoint = phrase boundary: commit the utterance and reset.
            enable_endpoint: true,
            rule1_min_trailing_silence: 2.4, // silence after any speech
            rule2_min_trailing_silence: 1.2, // silence after a decoded word
            rule3_min_utterance_length: 20.0, // hard cap on run-on speech
            ..Default::default()
        };
        let recognizer = OnlineRecognizer::create(&config)
            .ok_or("OnlineRecognizer::create failed (model load)")?;
        Ok(Self { recognizer })
    }

    pub fn new_channel(&self) -> ChannelStream {
        ChannelStream {
            stream: self.recognizer.create_stream(),
            last_pending: String::new(),
        }
    }

    /// Feed 16 kHz mono (or any rate — sherpa resamples internally) and pull
    /// whatever the recognizer can emit right now.
    pub fn feed(&self, ch: &mut ChannelStream, sample_rate: i32, samples: &[f32]) -> Emit {
        ch.stream.accept_waveform(sample_rate, samples);
        while self.recognizer.is_ready(&ch.stream) {
            self.recognizer.decode(&ch.stream);
        }
        let text = self
            .recognizer
            .get_result(&ch.stream)
            .map(|r| r.text)
            .unwrap_or_default();
        let text = text.trim().to_string();

        if self.recognizer.is_endpoint(&ch.stream) {
            self.recognizer.reset(&ch.stream);
            ch.last_pending.clear();
            if text.is_empty() {
                Emit::Nothing
            } else {
                Emit::Final(text)
            }
        } else if !text.is_empty() && text != ch.last_pending {
            ch.last_pending = text.clone();
            Emit::Pending(text)
        } else {
            Emit::Nothing
        }
    }
}

/// Model directory resolution, in priority order:
/// 1. `ANCHOR_ASR_MODEL_DIR` env (dev / power users)
/// 2. `<app_data>/models/<default name>` (first-run download target, Phase 8)
/// 3. the Phase-0 spike model, so dev works today without a downloader
pub fn model_dir(app_data: &Path) -> Option<PathBuf> {
    const DEFAULT: &str = "sherpa-onnx-nemotron-speech-streaming-en-0.6b-int8-2026-01-14";
    if let Ok(p) = std::env::var("ANCHOR_ASR_MODEL_DIR") {
        let p = PathBuf::from(p);
        if p.join("encoder.int8.onnx").exists() {
            return Some(p);
        }
    }
    let installed = app_data.join("models").join(DEFAULT);
    if installed.join("encoder.int8.onnx").exists() {
        return Some(installed);
    }
    // Dev fallback: the spike model checked in under the repo's ignored spike/.
    for up in ["..", "../..", "../../.."] {
        let candidate = Path::new(up)
            .join("spike/audio/models")
            .join(DEFAULT);
        if candidate.join("encoder.int8.onnx").exists() {
            return std::fs::canonicalize(candidate).ok();
        }
    }
    None
}
