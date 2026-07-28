//! ASR engine + model selection behind one interface (Handy-style: the user
//! picks from a few genuinely-good models, not a pile). The streaming Nemotron
//! primary (audio/asr.rs) runs either its multilingual or its faster EN-only
//! model; the offline Parakeet + LocalAgreement-2 fallback (audio/asr_offline.rs)
//! is the weak-CPU / compatibility path. Enum dispatch — not trait objects — so
//! the proven streaming path is untouched and the hot per-chunk `feed` stays a
//! direct call.

use super::asr::{self, Asr, ChannelStream, Emit};
use super::asr_offline::{self, LocalAgreementChannel, ParakeetAsr};
use std::path::{Path, PathBuf};

/// Which model to run. `Auto` = the multilingual streaming model when present,
/// else EN-only, else the offline fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnginePref {
    Auto,
    Multilingual,
    English,
    Offline,
}

impl EnginePref {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "offline" | "parakeet" | "compatibility" | "fallback" => EnginePref::Offline,
            "english" | "en" => EnginePref::English,
            // "streaming" is the legacy value for the streaming primary.
            "multilingual" | "multi" | "streaming" | "nemotron" => EnginePref::Multilingual,
            _ => EnginePref::Auto,
        }
    }
}

pub enum AsrEngine {
    /// Streaming Nemotron; the label distinguishes the multilingual vs EN model.
    Streaming(Asr, &'static str),
    Offline(ParakeetAsr),
}

pub enum AsrChannel {
    Streaming(ChannelStream),
    Offline(LocalAgreementChannel),
}

impl AsrEngine {
    /// Load the chosen model, falling back to whichever is actually installed.
    pub fn load(app_data: &Path, num_threads: i32, pref: EnginePref) -> Result<Self, String> {
        // The multilingual dir honours the ANCHOR_ASR_MODEL_DIR dev override.
        let multi: Option<PathBuf> = std::env::var("ANCHOR_ASR_MODEL_DIR")
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.join("encoder.int8.onnx").exists())
            .or_else(|| asr::model_dir_named(app_data, asr::ASR_MODEL_MULTILINGUAL));
        let english = asr::model_dir_named(app_data, asr::ASR_MODEL_EN);
        let offline = asr_offline::model_dir(app_data);

        let load_multi =
            |d: &Path| Asr::load(d, num_threads).map(|a| AsrEngine::Streaming(a, "multilingual"));
        let load_english =
            |d: &Path| Asr::load(d, num_threads).map(|a| AsrEngine::Streaming(a, "english"));
        let load_offline = |d: &Path| ParakeetAsr::load(d, num_threads).map(AsrEngine::Offline);

        match pref {
            EnginePref::English => {
                if let Some(d) = &english {
                    return load_english(d);
                }
                if let Some(d) = &multi {
                    return load_multi(d);
                }
                // Last resort: any installed model beats no audio at all. A user
                // whose only model is the offline one should still get a session.
                if let Some(d) = &offline {
                    return load_offline(d);
                }
            }
            EnginePref::Offline => {
                if let Some(d) = &offline {
                    return load_offline(d);
                }
                if let Some(d) = &multi {
                    return load_multi(d);
                }
                // Last resort: fall through to the English streaming model rather
                // than fail when it is the only one on disk.
                if let Some(d) = &english {
                    return load_english(d);
                }
            }
            EnginePref::Multilingual | EnginePref::Auto => {
                if let Some(d) = &multi {
                    return load_multi(d);
                }
                if let Some(d) = &english {
                    return load_english(d);
                }
                if let Some(d) = &offline {
                    return load_offline(d);
                }
            }
        }
        Err("No speech model installed. Open Settings → Speech model and download one, then start the call.".into())
    }

    pub fn label(&self) -> &'static str {
        match self {
            AsrEngine::Streaming(_, label) => label,
            AsrEngine::Offline(_) => "offline-parakeet",
        }
    }

    pub fn new_channel(&self, language: &str) -> AsrChannel {
        match self {
            AsrEngine::Streaming(a, _) => AsrChannel::Streaming(a.new_channel(language)),
            AsrEngine::Offline(p) => AsrChannel::Offline(p.new_channel(language)),
        }
    }

    pub fn feed(&self, ch: &mut AsrChannel, sample_rate: i32, samples: &[f32]) -> Emit {
        match (self, ch) {
            (AsrEngine::Streaming(a, _), AsrChannel::Streaming(c)) => a.feed(c, sample_rate, samples),
            (AsrEngine::Offline(p), AsrChannel::Offline(c)) => p.feed(c, sample_rate, samples),
            // Channels are always created by this same engine, so the arms above
            // are exhaustive in practice; a mismatch never occurs.
            _ => Emit::Nothing,
        }
    }
}
