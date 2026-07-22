//! Built-in fallback knowledge base (owner decision 2026-07-23): a small,
//! curated set of universal bridge anchors compiled into the binary —
//! invisible to the user, never stored in their DB. When an unexpected
//! question has no answer in the user's material, these give the model
//! honest, generic idea-anchors to bridge with (hobbies, strengths, small
//! talk) instead of silence or off-topic padding.
//!
//! Honesty contract: these are NOT the user's material. They are handed to
//! the model as hints it must mark [K]; they are deliberately excluded from
//! `Material::chunks()`, so the embedding post-check independently labels
//! anything derived from them as model-knowledge (violet).
//!
//! Speed contract: anchors are embedded once (lazy, cached); per question we
//! select only the few relevant lines, so the prompt grows by ~60 tokens,
//! not by the whole KB.

use crate::embed::Embedder;
use std::sync::OnceLock;

/// Universal bridge anchors. Keyword-dense, generic by design — they prompt
/// the speaker to fill in THEIR truth, never fabricate specifics for them.
const ANCHORS: &[&str] = &[
    // About yourself / small talk
    "Hobbies: name a real one — sport, books, side projects",
    "Outside work: walks, gym, cooking, learning something new",
    "Fun: building small things, games, time with family or friends",
    "Weekend: rest, a hobby, one thing you are learning",
    // Strengths / weaknesses
    "Strength: one real skill plus a concrete example",
    "Weakness: a real one, plus what you do about it",
    "Growth: something you improved in the last year",
    // Behavioural bridges
    "Conflict: listen first, restate their point, data over opinion",
    "Failure: own it, what changed after, lesson applied",
    "Pressure: prioritize, communicate early, one thing at a time",
    "Teamwork: unblock others, share credit, ask for help early",
    // Company / role
    "Why here: one thing about their product you genuinely liked",
    "Questions for them: team size, roadmap, how success is measured",
    "Motivation: growth, ownership, working with strong people",
    // Money / logistics (generic, no numbers)
    "Salary: market range researched, flexible for the right role",
    "Availability: be straight about notice period and start date",
    // Universal rescue
    "Ask them to clarify, then bridge to what you know",
    "Short honest answer beats a long invented one",
];

/// Anchor embeddings, computed once per process on first use (~0.3 s,
/// off the hot path — Mode 2 already runs on a background task).
fn index(embedder: &Embedder) -> Result<&'static Vec<(String, Vec<f32>)>, String> {
    static INDEX: OnceLock<Vec<(String, Vec<f32>)>> = OnceLock::new();
    if let Some(ix) = INDEX.get() {
        return Ok(ix);
    }
    let items: Vec<(String, String)> = ANCHORS
        .iter()
        .map(|a| (String::new(), a.to_string()))
        .collect();
    let vecs = embedder.embed_passages(&items)?;
    let built: Vec<(String, Vec<f32>)> = ANCHORS
        .iter()
        .map(|a| a.to_string())
        .zip(vecs)
        .collect();
    // A racing thread may have set it meanwhile; either value is identical.
    let _ = INDEX.set(built);
    Ok(INDEX.get().expect("fallback KB index just set"))
}

/// The anchors relevant to this question: cosine(question, anchor) above
/// `floor`, best-first, at most `k`. Empty when nothing fits — the ladder
/// then ends at the model's own [K] judgement, as before.
pub fn relevant(
    embedder: &Embedder,
    question_vec: &[f32],
    floor: f64,
    k: usize,
) -> Result<Vec<String>, String> {
    let ix = index(embedder)?;
    let mut scored: Vec<(f64, &String)> = ix
        .iter()
        .map(|(text, vec)| {
            let cos: f64 = question_vec
                .iter()
                .zip(vec)
                .map(|(x, y)| (x * y) as f64)
                .sum();
            (cos, text)
        })
        .filter(|(cos, _)| *cos >= floor)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored.into_iter().take(k).map(|(_, t)| t.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_stay_bullet_shaped() {
        for a in ANCHORS {
            assert!(
                a.split_whitespace().count() <= 12,
                "anchor drifted into prose: {a}"
            );
        }
    }
}
