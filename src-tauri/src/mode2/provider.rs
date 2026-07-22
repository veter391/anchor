//! Pluggable LLM providers for Mode-2 assembly. One trait, several impls
//! (Groq first, Ollama for fully-local). The trait returns raw bullet
//! strings; grounding/labelling happens in the parent module, provider-
//! agnostic. HTTP request/response shapes are filled per the verified API
//! contract (see 10_RESEARCH_LOG), never guessed.

use serde::Deserialize;

/// What the model is asked to produce: a titled set of short bullets.
#[derive(Debug, Clone, Deserialize)]
pub struct RawAssembly {
    pub title: String,
    pub bullets: Vec<String>,
}

/// A provider turns (question, material) into raw bullets. Async because it
/// is a network call; the local provider is async too for a uniform trait.
#[allow(async_fn_in_trait)]
pub trait Provider {
    async fn assemble(&self, prompt: &AssemblyPrompt) -> Result<RawAssembly, String>;
    fn name(&self) -> &'static str;
}

/// Bullet length modes (owner decision 2026-07-23): default = the current
/// keyword-dense style (locked — the owner likes it); short = 1–2 words;
/// long = slightly fuller. Shared by Mode-2 assembly and ingestion.
pub fn style_rule(style: &str) -> &'static str {
    match style {
        "short" => "Each bullet is 1-2 words only — a bare keyword anchor.",
        "long" => {
            "Each bullet may run up to ~15 words — still keywords and facts, never prose sentences."
        }
        _ => {
            "Each bullet is keyword-dense, ~10 words max: proper nouns, numbers, \
             plus-signs, commas — fast to scan, never a full sentence."
        }
    }
}

/// The strict, grounding-first instruction and the assembled context. Kept
/// as data so every provider sends the same thing.
pub struct AssemblyPrompt {
    pub question: String,
    pub material: String,
    /// Built-in universal bridge anchors relevant to this question (may be
    /// empty). NOT the user's material — the model must mark bullets drawn
    /// from these with [K].
    pub bridges: Vec<String>,
    /// Bullet length mode: "default" | "short" | "long" (see `style_rule`).
    pub style: String,
    pub max_bullets: usize,
}

impl AssemblyPrompt {
    /// System instruction — the grounding ladder, stated for the model.
    /// Owner decision (02_ARCHITECTURE §6): material first; only if the
    /// material does not cover it may the model use its own knowledge, and
    /// it must FLAG those bullets so the app can label them.
    pub fn system(&self) -> String {
        format!(
            "You assemble a tiny cue card for someone speaking live under pressure.\n\
             Output keyword bullets, never full sentences, never prose to read aloud.\n\
             SIZE THE CARD TO THE QUESTION: 2-3 bullets for a narrow fact, 4-5 for a normal \
             answer, up to {n} for a genuinely broad topic. A single bullet is only acceptable \
             when the only honest move is to ask for clarification.\n\
             {style}\n\
             RULE ZERO: every bullet must ANSWER the question just asked. Material that does not \
             address THIS question is off-limits — padding the card with unrelated notes is worse \
             than admitting the notes are silent.\n\
             GROUNDING LADDER (follow in order):\n\
             1. If the person's OWN MATERIAL below answers the question, use it. Prefer their exact \
             terms and numbers.\n\
             2. If the material does NOT answer this question, still help: use the UNIVERSAL BRIDGES \
             below (if provided) or a short honest answer from general knowledge, and mark each such \
             bullet by starting it with the token [K].\n\
             3. NEVER return an empty card. Blank is a failure. If you truly cannot help, return one \
             bullet: [K] Ask them to clarify, then bridge to what you know.\n\
             4. Never invent specifics (names, numbers, dates) you are not sure of. An honest general \
             point marked [K] is good; a fabricated specific is never acceptable.\n\
             Return ONLY the structured result: a short title (the question, rephrased) and the \
             bullets.",
            n = self.max_bullets,
            style = style_rule(&self.style)
        )
    }

    pub fn user(&self) -> String {
        let mut s = format!(
            "QUESTION JUST ASKED:\n{q}\n\nMY MATERIAL:\n{m}",
            q = self.question,
            m = if self.material.trim().is_empty() {
                "(none provided)"
            } else {
                &self.material
            }
        );
        if !self.bridges.is_empty() {
            s.push_str(
                "\n\nUNIVERSAL BRIDGES (not my material — if used, mark the bullet [K]):\n",
            );
            for b in &self.bridges {
                s.push_str("- ");
                s.push_str(b);
                s.push('\n');
            }
        }
        s
    }
}

/// The `[K]` token the system prompt tells the model to prefix
/// model-knowledge bullets with. Stripped after parsing; used as a hint that
/// combines with the embedding post-check.
pub const MODEL_KNOWLEDGE_TOKEN: &str = "[K]";

/// Splits a raw bullet into (is_model_flagged, clean_text).
pub fn strip_knowledge_flag(bullet: &str) -> (bool, String) {
    let t = bullet.trim();
    if let Some(rest) = t.strip_prefix(MODEL_KNOWLEDGE_TOKEN) {
        (true, rest.trim().to_string())
    } else {
        (false, t.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_flag_is_parsed_and_stripped() {
        assert_eq!(strip_knowledge_flag("[K] EU AI Act 2026"), (true, "EU AI Act 2026".into()));
        assert_eq!(strip_knowledge_flag("Internal platform, 26 agents"), (false, "Internal platform, 26 agents".into()));
    }
}
