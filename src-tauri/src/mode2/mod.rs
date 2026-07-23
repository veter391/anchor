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
    /// Stored embeddings parallel to `corpus_bullets` (from bullet_vec) —
    /// lets assembly rank bullets and the post-check skip re-embedding the
    /// corpus on every fire. `None` for bullets missing a vector.
    pub corpus_vecs: Vec<Option<Vec<f32>>>,
    /// Optional extra context the user attached to the session.
    pub cv: Option<String>,
    pub job_posting: Option<String>,
    pub research: Option<String>,
}

/// At most this many corpus bullets enter the assembly prompt, ranked by
/// relevance to the question — a big imported corpus must never overflow
/// the local model's context window (audit 2026-07-23).
const PROMPT_BULLET_CAP: usize = 48;

impl Material {
    fn extras(&self) -> impl Iterator<Item = &String> {
        [&self.cv, &self.job_posting, &self.research]
            .into_iter()
            .flatten()
    }

    /// The corpus bullets most relevant to the question, capped for the
    /// prompt. Bullets without a stored vector rank last (score 0).
    fn top_bullets(&self, qvec: &[f32], cap: usize) -> Vec<String> {
        let mut scored: Vec<(f64, &String)> = self
            .corpus_bullets
            .iter()
            .zip(self.corpus_vecs.iter().chain(std::iter::repeat(&None)))
            .map(|(text, vec)| {
                let s = vec.as_ref().map(|v| cosine(qvec, v)).unwrap_or(0.0);
                (s, text)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(cap).map(|(_, t)| t.clone()).collect()
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

/// Hard cap (owner decision 2026-07-23: up to 7-8 for genuinely broad
/// topics; the prompt scales the count to the question, this only caps it).
pub const MAX_BULLETS: usize = 8;

/// Below this many surviving bullets the card is too thin to help — top it
/// up with the strongest unused fallback bridges (labelled model-knowledge).
const MIN_USEFUL_BULLETS: usize = 3;

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
    // Reference vectors: STORED corpus embeddings (no per-fire re-embedding
    // of the whole corpus — audit 2026-07-23), plus one embed pass for any
    // bullet missing a vector and the free-text extras (CV/posting/research).
    let mut ref_vecs: Vec<Vec<f32>> = material.corpus_vecs.iter().flatten().cloned().collect();
    let mut to_embed: Vec<(String, String)> = material
        .corpus_bullets
        .iter()
        .zip(material.corpus_vecs.iter().chain(std::iter::repeat(&None)))
        .filter(|(_, v)| v.is_none())
        .map(|(t, _)| (String::new(), t.clone()))
        .collect();
    to_embed.extend(
        material
            .extras()
            .filter(|t| !t.trim().is_empty())
            .map(|t| (String::new(), t.clone())),
    );
    if !to_embed.is_empty() {
        ref_vecs.extend(embedder.embed_passages(&to_embed)?);
    }

    let bullets = if ref_vecs.is_empty() {
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
        let mut out = Vec::new();
        for bt in raw_bullets.iter().take(MAX_BULLETS) {
            let bvec = embedder.embed_query(bt)?;
            let best = ref_vecs
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
    style: &str,
) -> Result<AssembledCard, String> {
    // One question embedding, reused by the fallback-KB selection here and
    // the relevance gate after generation.
    let qvec = embedder.embed_query(question)?;
    // Built-in universal bridges relevant to THIS question (owner decision:
    // the card must help even when the user's material is silent). They are
    // hints the model must mark [K]; they never enter the post-check refs, so
    // the post-check labels anything derived from them as model-knowledge.
    let bridges = fallback_kb::relevant(embedder, &qvec, REL_FLOOR, 6).unwrap_or_default();

    // Only the question-relevant slice of the corpus enters the prompt — a
    // large imported corpus must never overflow the local context window.
    let prompt_bullets = material.top_bullets(&qvec, PROMPT_BULLET_CAP);
    let material_text = material_to_prompt(&prompt_bullets, material);
    let prompt = AssemblyPrompt {
        question: question.to_string(),
        material: material_text,
        bridges: bridges.clone(),
        style: style.to_string(),
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
    // Too thin a card helps nobody (owner: "one bullet is not an answer").
    // Top up with the strongest unused fallback bridges — they were already
    // selected by relevance to THIS question, and they carry the [K] flag.
    if texts.len() < MIN_USEFUL_BULLETS {
        for b in &bridges {
            if texts.len() >= MIN_USEFUL_BULLETS {
                break;
            }
            let dup = texts.iter().any(|t| crate::audio::text_overlap(t, b) >= 0.6);
            if !dup {
                texts.push(b.clone());
                flags.push(true);
            }
        }
    }
    if texts.is_empty() {
        // Nothing the model produced answers the question and no bridge fits
        // — the honest bottom of the ladder, enforced in code.
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

/// Generic completion on whichever provider is configured. Used by the
/// ingestion card generator; Mode-2 assembly keeps its dedicated path.
/// Returns raw model text (JSON expected inside; caller extracts).
pub async fn complete(
    choice: &ProviderChoice,
    system: String,
    user: String,
    max_tokens: usize,
) -> Result<String, String> {
    match choice {
        ProviderChoice::Api {
            provider,
            api_key,
            model,
            custom_base_url,
        } => {
            let compat = if provider == "custom" {
                let base = custom_base_url.clone().ok_or("custom provider needs a base URL")?;
                openai_compat::OpenAiCompat::custom(
                    base,
                    api_key.clone(),
                    model.clone().unwrap_or_default(),
                )
            } else {
                openai_compat::OpenAiCompat::preset(provider, api_key.clone(), model.clone())
            };
            compat.complete_json(&system, &user, max_tokens as u32).await
        }
        ProviderChoice::Local {
            engine,
            model_id,
            model_path,
        } => {
            engine.ensure(model_id, model_path)?;
            let engine = engine.clone();
            tauri::async_runtime::spawn_blocking(move || {
                engine.complete(&system, &user, max_tokens)
            })
            .await
            .map_err(|e| format!("local completion task failed: {e}"))?
        }
    }
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

fn material_to_prompt(bullets: &[String], m: &Material) -> String {
    let mut s = String::new();
    if !bullets.is_empty() {
        s.push_str("Prepared notes:\n");
        for b in bullets {
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
