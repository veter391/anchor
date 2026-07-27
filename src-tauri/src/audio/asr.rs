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

    /// A per-channel stream. `language` steers the multilingual model — an ISO
    /// code like "en"/"de"/"ru", or "auto"/"" for auto-detection (the model
    /// picks the language from the audio). Ignored by the EN-only fallback model.
    pub fn new_channel(&self, language: &str) -> ChannelStream {
        let stream = self.recognizer.create_stream();
        let lang = language.trim();
        if !lang.is_empty() && lang != "auto" {
            stream.set_option("language", lang);
        }
        ChannelStream {
            stream,
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
/// 2. `<app_data>/models/<name>` (first-run download target, Phase 8)
/// 3. the Phase-0 spike dir, so dev works today without a downloader
///
/// Preferred model is the MULTILINGUAL Nemotron 3.5 (EN/ES/RU/UK/DE + more, with
/// per-stream language selection / auto-detect — Phase 7). The EN-only spike
/// model is kept as a fallback so dev never breaks while the new one downloads.
pub const ASR_MODEL_MULTILINGUAL: &str =
    "sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-320ms-int8-2026-06-11";
pub const ASR_MODEL_EN: &str = "sherpa-onnx-nemotron-speech-streaming-en-0.6b-int8-2026-01-14";

pub fn model_dir(app_data: &Path) -> Option<PathBuf> {
    // 1. Explicit override wins.
    if let Ok(p) = std::env::var("ANCHOR_ASR_MODEL_DIR") {
        let p = PathBuf::from(p);
        if p.join("encoder.int8.onnx").exists() {
            return Some(p);
        }
    }
    // 2/3. Installed (portable data), then the dev spike dir — multilingual first.
    for name in [ASR_MODEL_MULTILINGUAL, ASR_MODEL_EN] {
        let installed = app_data.join("models").join(name);
        if installed.join("encoder.int8.onnx").exists() {
            return Some(installed);
        }
        for up in ["..", "../..", "../../.."] {
            let candidate = Path::new(up).join("spike/audio/models").join(name);
            if candidate.join("encoder.int8.onnx").exists() {
                return std::fs::canonicalize(candidate).ok();
            }
        }
    }
    None
}
