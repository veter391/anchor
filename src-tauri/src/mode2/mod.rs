//! Mode 2 — live card assembly. Fires when Level-1 retrieval has no
//! confident match (matcher::Decision::NoConfidence): an unprepared question
//! came up. We assemble a small card from the user's OWN material first, and
//! only reach beyond it when the material does not cover the question — and
//! every such bullet is labelled. See Documents/00_PRODUCT.md §Level-2 and
//! 02_ARCHITECTURE.md §6 (the grounding ladder).
//!
//! The provider is pluggable (Groq first, Ollama for fully-local); the
//! grounding ladder, the post-check, and the panic card live here and do not
//! depend on which provider answered.

pub mod fallback_kb;
pub mod local;
pub mod models;
pub mod openai_compat;
pub mod provider;

use crate::embed::Embedder;
use provider::{strip_knowledge_flag, AssemblyPrompt, Provider};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Where a bullet came from — drives the overlay treatment (prepared vs
/// amber assembled vs violet model-knowledge). Never blur this line.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Assembled and confirmed to be grounded in the user's material.
    AssembledGrounded,
    /// The material did not cover it; this came from the model's knowledge.
    ModelKnowledge,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssembledBullet {
    pub text: String,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssembledCard {
    pub title: String,
    /// "assembled" (amber, built live from material) or "unexpected" (calm
    /// amber-orange, the fixed universal anchors). Drives the overlay treatment.
    pub source: &'static str,
    pub bullets: Vec<AssembledBullet>,
    /// True if any bullet is ModelKnowledge (for the card-level notice).
    pub has_model_knowledge: bool,
}

/// The user's material handed to the model, and used as the grounding
/// reference for the post-check.
#[derive(Debug, Clone, Default)]
pub struct Material {
    /// Every prepared bullet across the corpus (the primary grounding source).
    pub corpus_bullets: Vec<String>,
    /// Optional extra context the user attached to the session.
    pub cv: Option<String>,
    pub job_posting: Option<String>,
    pub research: Option<String>,
}

impl Material {
    /// All grounding text as one bag of chunks for the post-check.
    fn chunks(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.corpus_bullets.iter().map(|s| s.as_str()).collect();
        for extra in [&self.cv, &self.job_posting, &self.research].into_iter().flatten() {
            v.push(extra.as_str());
        }
        v
    }
}

/// Post-check floor: a generated bullet whose best cosine to any material
/// chunk is below this is treated as NOT grounded (labelled model-knowledge,
/// not silently presented as the user's). Tuned on real data in Phase 7;
/// 0.55 matches the bullet-coverage floor as a sane start.
pub const GROUND_FLOOR: f64 = 0.55;

/// Question-relevance floor: a bullet whose cosine(question-as-query,
/// bullet-as-passage) is below this does not ANSWER the question and is
/// dropped — no matter how well it matches the material. Calibrated
/// 2026-07-23 on `examples/rel_probe.rs` (relevant min 0.333 vs irrelevant
/// max 0.309 across EN + cross-lingual RU pairs); re-tune in Phase 7.
pub const REL_FLOOR: f64 = 0.32;

pub const MAX_BULLETS: usize = 6;

/// The panic card — shown instantly on the hotkey, and as the filler while
/// Mode 2 is still assembling. Fixed, universal, no generation.
pub fn panic_card() -> AssembledCard {
    AssembledCard {
        title: "Take a breath".into(),
        source: "unexpected",
        bullets: [
            "Ask them to clarify the question",
            "Structure: context, action, result",
            "Bridge to a project you actually built",
        ]
        .iter()
        .map(|t| AssembledBullet {
            text: t.to_string(),
            provenance: Provenance::AssembledGrounded,
        })
        .collect(),
        has_model_knowledge: false,
    }
}

/// Runs the post-check: label each generated bullet by whether it is grounded
/// in the supplied material. Embedding-similarity is a cheap grounding gate
/// (02_ARCHITECTURE §6) — it catches bullets with no source. It is a labeller
/// here, not a hard filter: an ungrounded bullet is marked model-knowledge,
/// not dropped, because the owner's ladder allows a labelled fallback.
pub fn ground_check(
    embedder: &Embedder,
    material: &Material,
    raw_bullets: &[String],
    title: &str,
) -> Result<AssembledCard, String> {
    let chunks = material.chunks();
    let bullets = if chunks.is_empty() {
        // No material to check against → everything is model-knowledge.
        raw_bullets
            .iter()
            .take(MAX_BULLETS)
            .map(|t| AssembledBullet {
                text: t.clone(),
                provenance: Provenance::ModelKnowledge,
            })
            .collect()
    } else {
        // Embed material chunks once and each bullet, compare max cosine.
        let chunk_items: Vec<(String, String)> =
            chunks.iter().map(|c| (String::new(), c.to_string())).collect();
        let chunk_vecs = embedder.embed_passages(&chunk_items)?;

        let mut out = Vec::new();
        for bt in raw_bullets.iter().take(MAX_BULLETS) {
            let bvec = embedder.embed_query(bt)?;
            let best = chunk_vecs
                .iter()
                .map(|cv| cosine(&bvec, cv))
                .fold(f64::MIN, f64::max);
            let provenance = if best >= GROUND_FLOOR {
                Provenance::AssembledGrounded
            } else {
                Provenance::ModelKnowledge
            };
            out.push(AssembledBullet {
                text: bt.clone(),
                provenance,
            });
        }
        out
    };

    let has_model_knowledge = bullets
        .iter()
        .any(|b| b.provenance == Provenance::ModelKnowledge);
    Ok(AssembledCard {
        title: title.to_string(),
        source: "assembled",
        bullets,
        has_model_knowledge,
    })
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    // Vectors are unit-normalized by the embedder → cosine is the dot product.
    a.iter().zip(b).map(|(x, y)| (x * y) as f64).sum()
}

/// The enforced bottom of the grounding ladder: every generated bullet was
/// off-question, so the card says the one honest thing instead.
fn clarify_card(title: &str) -> AssembledCard {
    AssembledCard {
        title: title.to_string(),
        source: "assembled",
        bullets: vec![AssembledBullet {
            text: "Ask them to clarify, then bridge to what you know".into(),
            provenance: Provenance::ModelKnowledge,
        }],
        has_model_knowledge: true,
    }
}

// ── Provider selection + the full assembly path ─────────────────────

/// Which provider to use for Mode-2 assembly, chosen in settings.
/// - Api: any OpenAI-compatible endpoint (OpenRouter default / Groq / OpenAI /
///   custom), BYOK.
/// - Local: an embedded GGUF model (the free path), by registry id.
#[derive(Clone)]
pub enum ProviderChoice {
    Api {
        provider: String,
        api_key: String,
        model: Option<String>,
        custom_base_url: Option<String>,
    },
    Local {
        engine: Arc<local::LocalEngine>,
        model_id: String,
        model_path: std::path::PathBuf,
    },
}

/// End to end: ask the provider, then run the grounding ladder. The provider's
/// `[K]` flag AND the embedding post-check both feed the label — a bullet is
/// grounded only if the model did not flag it AND it has a real source in the
/// material. This is the belt-and-suspenders that makes the guarantee real.
pub async fn assemble(
    choice: &ProviderChoice,
    embedder: &Embedder,
    material: &Material,
    question: &str,
) -> Result<AssembledCard, String> {
    // One question embedding, reused by the fallback-KB selection here and
    // the relevance gate after generation.
    let qvec = embedder.embed_query(question)?;
    // Built-in universal bridges relevant to THIS question (owner decision:
    // the card must help even when the user's material is silent). They are
    // hints the model must mark [K]; they never enter Material::chunks(), so
    // the post-check labels anything derived from them as model-knowledge.
    let bridges = fallback_kb::relevant(embedder, &qvec, REL_FLOOR, 6).unwrap_or_default();

    let material_text = material_to_prompt(material);
    let prompt = AssemblyPrompt {
        question: question.to_string(),
        material: material_text,
        bridges,
        max_bullets: MAX_BULLETS,
    };

    let raw = match choice {
        ProviderChoice::Api {
            provider,
            api_key,
            model,
            custom_base_url,
        } => {
            if provider == "custom" {
                let base = custom_base_url.clone().ok_or("custom provider needs a base URL")?;
                openai_compat::OpenAiCompat::custom(
                    base,
                    api_key.clone(),
                    model.clone().unwrap_or_default(),
                )
                .assemble(&prompt)
                .await?
            } else {
                openai_compat::OpenAiCompat::preset(provider, api_key.clone(), model.clone())
                    .assemble(&prompt)
                    .await?
            }
        }
        ProviderChoice::Local {
            engine,
            model_id,
            model_path,
        } => {
            local::Local::new(engine.clone(), model_id.clone(), model_path.clone())
                .assemble(&prompt)
                .await?
        }
    };

    // Split each bullet's model-knowledge flag from its text, then post-check.
    let mut flags = Vec::with_capacity(raw.bullets.len());
    let mut texts = Vec::with_capacity(raw.bullets.len());
    for b in &raw.bullets {
        let (flagged, clean) = strip_knowledge_flag(b);
        flags.push(flagged);
        texts.push(clean);
    }

    // Relevance gate BEFORE grounding: a bullet that does not address the
    // question is dropped even if it matches the material perfectly (the
    // owner-caught failure: an off-topic question answered with confident
    // but unrelated "your material" bullets).
    if !texts.is_empty() {
        let passage_items: Vec<(String, String)> =
            texts.iter().map(|t| (String::new(), t.clone())).collect();
        let bvecs = embedder.embed_passages(&passage_items)?;
        let mut kept_texts = Vec::with_capacity(texts.len());
        let mut kept_flags = Vec::with_capacity(flags.len());
        for ((text, flag), bvec) in texts.iter().zip(&flags).zip(&bvecs) {
            let rel = cosine(&qvec, bvec);
            if rel >= REL_FLOOR {
                kept_texts.push(text.clone());
                kept_flags.push(*flag);
            } else {
                tracing::info!(rel, bullet = %text, "mode2: dropped off-question bullet");
            }
        }
        texts = kept_texts;
        flags = kept_flags;
    }
    if texts.is_empty() {
        // Nothing the model produced answers the question — the honest
        // bottom of the ladder, enforced in code, never a fabricated card.
        return Ok(clarify_card(&raw.title));
    }

    let mut card = ground_check(embedder, material, &texts, &raw.title)?;
    // The model's own [K] flag is authoritative for "the model knew it was
    // reaching beyond the notes"; OR it with the embedding verdict.
    for (bullet, model_flagged) in card.bullets.iter_mut().zip(flags) {
        if model_flagged {
            bullet.provenance = Provenance::ModelKnowledge;
        }
    }
    card.has_model_knowledge = card
        .bullets
        .iter()
        .any(|b| b.provenance == Provenance::ModelKnowledge);
    Ok(card)
}

// ── API key storage in the OS keyring (never our DB) ────────────────

const KEYRING_SERVICE: &str = "anchor-llm";

pub fn keyring_set(provider: &str, key: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, provider)
        .and_then(|e| e.set_password(key))
        .map_err(|e| e.to_string())
}

pub fn keyring_get(provider: &str) -> Result<String, String> {
    keyring::Entry::new(KEYRING_SERVICE, provider)
        .and_then(|e| e.get_password())
        .map_err(|e| e.to_string())
}

pub fn keyring_delete(provider: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, provider)
        .and_then(|e| e.delete_credential())
        .map_err(|e| e.to_string())
}

pub fn keyring_has(provider: &str) -> bool {
    keyring_get(provider).is_ok()
}

fn material_to_prompt(m: &Material) -> String {
    let mut s = String::new();
    if !m.corpus_bullets.is_empty() {
        s.push_str("Prepared notes:\n");
        for b in &m.corpus_bullets {
            s.push_str("- ");
            s.push_str(b);
            s.push('\n');
        }
    }
    for (label, extra) in [
        ("CV", &m.cv),
        ("Job posting", &m.job_posting),
        ("Company research", &m.research),
    ] {
        if let Some(text) = extra {
            if !text.trim().is_empty() {
                s.push_str(&format!("\n{label}:\n{text}\n"));
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_card_is_three_fixed_anchors_no_model_knowledge() {
        let c = panic_card();
        assert_eq!(c.bullets.len(), 3);
        assert!(!c.has_model_knowledge);
        assert!(c.bullets.iter().all(|b| b.provenance == Provenance::AssembledGrounded));
    }
}
