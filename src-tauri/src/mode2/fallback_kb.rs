//! Built-in fallback knowledge base (owner decision 2026-07-23): a small,
//! curated set of universal bridge anchors compiled into the binary —
//! invisible to the user, never stored in their DB. When an unexpected
//! question has no answer in the user's material, these give the model
//! honest, generic idea-anchors to bridge with (hobbies, strengths, small
//! talk) instead of silence or off-topic padding.
//!
//! Honesty contract: these are NOT the user's material. They are handed to
//! the model as hints it must mark [K]; they are deliberately excluded from
//! the post-check's reference vectors, so anything derived from them is
//! independently labelled model-knowledge (violet).
//!
//! Speed contract: anchors are embedded once (lazy, cached); per question we
//! select only the few relevant lines, so the prompt grows by ~60 tokens,
//! not by the whole KB.

use crate::embed::Embedder;
use std::sync::OnceLock;

/// Universal bridge anchors. Keyword-dense but SUBSTANTIVE — each one is a
/// usable answer skeleton, not a banal noun. They prompt the speaker to fill
/// in THEIR truth, never fabricate specifics for them. The user's own
/// material always outranks these (grounding ladder step 1); these exist so
/// an uncovered question still gets a professional way out.
const ANCHORS: &[&str] = &[
    // ── Free time / personality (the "what do you do for fun" class) ──
    "Pick one real hobby + one detail: sport 3x week, chess online, photography walks",
    "Something you build or learn off-hours: side project, course, language — shows drive",
    "Balance formula: one active thing + one calm thing + people you do it with",
    "Tie the hobby back: endurance sport = discipline, games = strategy, cooking = patience",
    "Tell me about yourself: present role, one proud result, why this conversation",
    // ── Strengths / weaknesses / growth ──
    "Strength = skill + proof: name it, then one concrete result with a number",
    "Weakness formula: real one + the system you built around it + progress since",
    "Pick a growth story from last year: what you couldn't do, what changed, how",
    // ── Behavioural (conflict, failure, pressure, leadership) ──
    "Conflict: listen fully, restate their view first, bring data, agree on the test",
    "Failure story spine: my call, what broke, what I owned, the rule I keep since",
    "Under pressure: cut scope, name the one priority aloud, communicate early and often",
    "Deadline slipping: flag it the day you know, offer two options with costs",
    "Leading without title: unblock others first, share credit loudly, take blame quietly",
    // ── Company / role / motivation ──
    "Why us: one genuine thing about their product + one thing you would improve",
    "Why this role: the overlap between what you are best at and what they need",
    "Motivation ladder: ownership, hard problems, strong people to learn from — pick honestly",
    "Questions to ask them: how success is measured, biggest current fire, team rituals",
    // ── Money / logistics (generic, never invent numbers) ──
    "Salary: name the researched market range, anchor on total comp, stay flexible on mix",
    "Counter-question: what does the band look like for this level here?",
    "Availability: exact notice period, honest start date, no fake urgency",
    // ── Universal rescue (any question, any topic) ──
    "Buy 5 seconds: repeat the question in your own words — 'so you are asking…'",
    "Any answer spine: context in one line, what I did, the result, the lesson",
    "Unknown territory: say which part you DO know, answer that, ask if it helps",
    "Honest gap: 'not used X in production — here is the closest thing I did'",
    "Bridge move: answer the neighbouring question you are strong in, then check back",
    "Never bluff specifics: a confident wrong number costs more than an honest range",
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
            let words = a.split_whitespace().count();
            assert!(
                (4..=16).contains(&words),
                "anchor must be a substantive one-liner (4-16 words), got {words}: {a}"
            );
        }
    }
}
