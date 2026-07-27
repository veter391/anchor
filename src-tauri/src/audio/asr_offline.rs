//! Offline fallback ASR: NVIDIA Parakeet-TDT-0.6b-v3 (int8, 25 European languages
//! incl. ES/RU/UK) via sherpa-onnx `OfflineRecognizer`, made pseudo-streaming
//! with **LocalAgreement-2**.
//!
//! The primary path is the streaming Nemotron (audio/asr.rs). This is the
//! contingency (04_MODELS §1) for CPUs where the streaming model misses the RTF
//! budget: Parakeet int8 decodes ~30× realtime, so re-decoding a short growing
//! buffer every ~400 ms is cheap. `OfflineRecognizer` has no built-in streaming
//! (sherpa-onnx issue #2918), so we do it ourselves, exactly the ufal/whisper-
//! streaming policy: re-decode the buffer, and only emit the prefix on which two
//! consecutive decodes AGREE (that prefix is stable and won't be revised). A
//! trailing-silence endpoint commits the utterance and resets the buffer, which
//! also bounds the re-decode cost. The upward contract is the same `Emit` the
//! streaming path uses, so the worker and match engine don't care which is live.

use super::asr::Emit;
use sherpa_onnx::{
    OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig,
};
use std::path::{Path, PathBuf};

pub const ASR_MODEL_PARAKEET: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";

/// Re-decode the growing buffer this often while speech is ongoing.
const REDECODE_MS: usize = 400;
/// Trailing silence that commits the utterance (phrase boundary).
const ENDPOINT_SILENCE_MS: usize = 1200;
/// Hard cap on a run-on utterance, so the buffer (and re-decode cost) is bounded.
const MAX_UTTERANCE_S: usize = 20;
/// Don't decode until there's at least this much audio (avoids noise on a syllable).
const MIN_DECODE_MS: usize = 300;
/// RMS below this over a chunk counts as silence.
const SILENCE_RMS: f32 = 1e-3;

pub struct ParakeetAsr {
    recognizer: OfflineRecognizer,
}

/// Per-channel pseudo-streaming state.
pub struct LocalAgreementChannel {
    buffer: Vec<f32>,
    prev_hyp: String,
    emitted: String,
    since_decode: usize,
    trailing_silence: usize,
    has_speech: bool,
    language: String,
}

impl ParakeetAsr {
    pub fn load(model_dir: &Path, num_threads: i32) -> Result<Self, String> {
        let enc = model_dir.join("encoder.int8.onnx");
        let dec = model_dir.join("decoder.int8.onnx");
        let joi = model_dir.join("joiner.int8.onnx");
        let tok = model_dir.join("tokens.txt");
        for p in [&enc, &dec, &joi, &tok] {
            if !p.exists() {
                return Err(format!("missing fallback ASR file: {}", p.display()));
            }
        }
        let config = OfflineRecognizerConfig {
            model_config: OfflineModelConfig {
                transducer: OfflineTransducerModelConfig {
                    encoder: Some(enc.to_string_lossy().into_owned()),
                    decoder: Some(dec.to_string_lossy().into_owned()),
                    joiner: Some(joi.to_string_lossy().into_owned()),
                },
                tokens: Some(tok.to_string_lossy().into_owned()),
                model_type: Some("nemo_transducer".to_string()),
                num_threads,
                provider: Some("cpu".to_string()),
                debug: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or("OfflineRecognizer::create failed (fallback model load)")?;
        Ok(Self { recognizer })
    }

    pub fn new_channel(&self, language: &str) -> LocalAgreementChannel {
        LocalAgreementChannel {
            buffer: Vec::new(),
            prev_hyp: String::new(),
            emitted: String::new(),
            since_decode: 0,
            trailing_silence: 0,
            has_speech: false,
            language: language.trim().to_string(),
        }
    }

    /// Same contract as the streaming path: a growing `Pending` partial and a
    /// `Final` at each phrase boundary.
    pub fn feed(&self, ch: &mut LocalAgreementChannel, sample_rate: i32, samples: &[f32]) -> Emit {
        let sr = sample_rate.max(1) as usize;
        let silent = is_silent(samples);

        // Skip leading silence so the buffer holds a speech segment, not dead air.
        if !ch.has_speech {
            if silent {
                return Emit::Nothing;
            }
            ch.has_speech = true;
        }
        ch.buffer.extend_from_slice(samples);
        ch.since_decode += samples.len();
        if silent {
            ch.trailing_silence += samples.len();
        } else {
            ch.trailing_silence = 0;
        }

        let endpoint = sr * ENDPOINT_SILENCE_MS / 1000;
        let max_len = sr * MAX_UTTERANCE_S;
        let redecode = sr * REDECODE_MS / 1000;
        let min_len = sr * MIN_DECODE_MS / 1000;

        // Phrase boundary (or a run-on hitting the cap): commit and reset.
        if ch.trailing_silence >= endpoint || ch.buffer.len() >= max_len {
            let text = self.decode(&ch.buffer, sample_rate, &ch.language);
            ch.reset();
            return if text.is_empty() {
                Emit::Nothing
            } else {
                Emit::Final(text)
            };
        }

        // Mid-utterance: re-decode and emit only the newly-agreed prefix.
        if ch.since_decode >= redecode && ch.buffer.len() >= min_len {
            ch.since_decode = 0;
            let hyp = self.decode(&ch.buffer, sample_rate, &ch.language);
            let stable = agreed_prefix(&ch.prev_hyp, &hyp);
            ch.prev_hyp = hyp;
            if !stable.is_empty() && stable.len() > ch.emitted.len() {
                ch.emitted = stable.clone();
                return Emit::Pending(stable);
            }
        }
        Emit::Nothing
    }

    fn decode(&self, buffer: &[f32], sample_rate: i32, language: &str) -> String {
        let stream = self.recognizer.create_stream();
        if !language.is_empty() && language != "auto" {
            stream.set_option("language", language);
        }
        stream.accept_waveform(sample_rate, buffer);
        self.recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default()
    }
}

impl LocalAgreementChannel {
    fn reset(&mut self) {
        self.buffer.clear();
        self.prev_hyp.clear();
        self.emitted.clear();
        self.since_decode = 0;
        self.trailing_silence = 0;
        self.has_speech = false;
    }
}

/// LocalAgreement-2: the longest word-prefix on which two consecutive decodes
/// agree. Because a later decode rarely rewrites an already-stable prefix, that
/// prefix is safe to emit as confirmed.
fn agreed_prefix(prev: &str, curr: &str) -> String {
    let pw: Vec<&str> = prev.split_whitespace().collect();
    let cw: Vec<&str> = curr.split_whitespace().collect();
    let n = pw
        .iter()
        .zip(cw.iter())
        .take_while(|(a, b)| a == b)
        .count();
    cw[..n].join(" ")
}

fn is_silent(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return true;
    }
    let ms = samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32;
    ms.sqrt() < SILENCE_RMS
}

/// Fallback-model directory resolution, mirroring `asr::model_dir`:
/// `ANCHOR_ASR_FALLBACK_DIR` env, then `<app_data>/models/<name>`, then the dev
/// spike dir.
pub fn model_dir(app_data: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ANCHOR_ASR_FALLBACK_DIR") {
        let p = PathBuf::from(p);
        if p.join("encoder.int8.onnx").exists() {
            return Some(p);
        }
    }
    let installed = app_data.join("models").join(ASR_MODEL_PARAKEET);
    if installed.join("encoder.int8.onnx").exists() {
        return Some(installed);
    }
    for up in ["..", "../..", "../../.."] {
        let candidate = Path::new(up)
            .join("spike/audio/models")
            .join(ASR_MODEL_PARAKEET);
        if candidate.join("encoder.int8.onnx").exists() {
            return std::fs::canonicalize(candidate).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::agreed_prefix;

    #[test]
    fn agreed_prefix_is_the_common_word_prefix() {
        // The stable prefix grows as decodes agree further.
        assert_eq!(agreed_prefix("why are you", "why are you leaving"), "why are you");
        // Divergence stops the confirmed prefix at the last agreed word.
        assert_eq!(
            agreed_prefix("why are you leaving now", "why are you moving on"),
            "why are you"
        );
        // No agreement / empties.
        assert_eq!(agreed_prefix("", "hello there"), "");
        assert_eq!(agreed_prefix("hello there", ""), "");
        assert_eq!(agreed_prefix("apples", "oranges"), "");
        // Full agreement returns the whole thing.
        assert_eq!(agreed_prefix("same exact text", "same exact text"), "same exact text");
    }
}
