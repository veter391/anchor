//! ASR engine selection behind one interface: the streaming Nemotron primary
//! (audio/asr.rs) or the offline Parakeet + LocalAgreement-2 fallback
//! (audio/asr_offline.rs). Enum dispatch — not trait objects — so the proven
//! streaming path is untouched and the hot per-chunk `feed` stays a direct call.

use super::asr::{Asr, ChannelStream, Emit};
use super::asr_offline::{LocalAgreementChannel, ParakeetAsr};
use std::path::Path;

/// Which engine to run. `Auto` = the streaming primary when its model is present,
/// else the fallback. `Offline` forces the fallback (weak CPUs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnginePref {
    Auto,
    Streaming,
    Offline,
}

impl EnginePref {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "offline" | "parakeet" | "fallback" => EnginePref::Offline,
            "streaming" | "nemotron" | "primary" => EnginePref::Streaming,
            _ => EnginePref::Auto,
        }
    }
}

pub enum AsrEngine {
    Streaming(Asr),
    Offline(ParakeetAsr),
}

pub enum AsrChannel {
    Streaming(ChannelStream),
    Offline(LocalAgreementChannel),
}

impl AsrEngine {
    /// Load per the preference, falling back to whichever model is actually
    /// installed if the preferred one is missing.
    pub fn load(app_data: &Path, num_threads: i32, pref: EnginePref) -> Result<Self, String> {
        let stream = super::asr::model_dir(app_data);
        let offline = super::asr_offline::model_dir(app_data);
        match pref {
            EnginePref::Offline => {
                if let Some(d) = &offline {
                    return ParakeetAsr::load(d, num_threads).map(AsrEngine::Offline);
                }
                if let Some(d) = &stream {
                    return Asr::load(d, num_threads).map(AsrEngine::Streaming);
                }
            }
            EnginePref::Streaming | EnginePref::Auto => {
                if let Some(d) = &stream {
                    return Asr::load(d, num_threads).map(AsrEngine::Streaming);
                }
                if let Some(d) = &offline {
                    return ParakeetAsr::load(d, num_threads).map(AsrEngine::Offline);
                }
            }
        }
        Err("no ASR model found — set ANCHOR_ASR_MODEL_DIR / ANCHOR_ASR_FALLBACK_DIR or install a model".into())
    }

    pub fn label(&self) -> &'static str {
        match self {
            AsrEngine::Streaming(_) => "streaming-nemotron",
            AsrEngine::Offline(_) => "offline-parakeet",
        }
    }

    pub fn new_channel(&self, language: &str) -> AsrChannel {
        match self {
            AsrEngine::Streaming(a) => AsrChannel::Streaming(a.new_channel(language)),
            AsrEngine::Offline(p) => AsrChannel::Offline(p.new_channel(language)),
        }
    }

    pub fn feed(&self, ch: &mut AsrChannel, sample_rate: i32, samples: &[f32]) -> Emit {
        match (self, ch) {
            (AsrEngine::Streaming(a), AsrChannel::Streaming(c)) => a.feed(c, sample_rate, samples),
            (AsrEngine::Offline(p), AsrChannel::Offline(c)) => p.feed(c, sample_rate, samples),
            // Channels are always created by this same engine, so the arms above
            // are exhaustive in practice; a mismatch never occurs.
            _ => Emit::Nothing,
        }
    }
}
