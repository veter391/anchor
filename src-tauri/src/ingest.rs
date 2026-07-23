//! Raw-material ingestion (owner decision 2026-07-23): users dump
//! unstructured study text — notes, knowledge bases, whole documents — and
//! the app turns it into draft anchor cards with the configured Mode-2
//! provider (local model by default). Hybrid by design: drafts land in the
//! import box for review/edit, or go straight into the corpus in auto mode.
//! Either way the drafts pass through the SAME markdown contract and parser
//! as hand-written cards — one format, one validation path.

use crate::mode2::provider::style_rule;
use crate::mode2::{self, ProviderChoice};
use crate::textfmt::{derive_short, tighten_default};
use serde::Deserialize;

/// Canonical bullets are ALWAYS generated in the Recommended (default) style —
/// it is the storage baseline; the short/long variants derive from it and the
/// display setting only chooses which to show. So ingestion ignores the
/// current display style for generation and uses the default keyword rule.
fn system_prompt() -> String {
    format!(
        "You turn a person's own study material into cue cards for speaking under pressure.\n\
         A card = one question-style title (phrased the way a person would ASK it in a call) \
         plus 3-6 anchor bullets. The title must accurately describe what its bullets say — \
         never attach one thing's title to another thing's facts.\n\
         {style}\n\
         Bullets are keyword fragments, NOT sentences: no articles, no 'is/are', drop filler. \
         Good: \"40 services, zero downtime\", \"Helm + ArgoCD, GitOps\". \
         Bad: \"We moved forty services with zero downtime\".\n\
         Use ONLY facts present in the material — never invent names, numbers or claims. \
         Skip filler; if a passage holds nothing worth anchoring, produce fewer cards.\n\
         Keep the material's own language for titles and bullets.\n\
         Return ONLY a JSON object: {{\"cards\": [{{\"title\": \"...\", \"points\": [\"...\"]}}]}} \
         with 1-3 cards.",
        style = style_rule("default")
    )
}

#[derive(Deserialize)]
struct DraftCards {
    cards: Vec<DraftCard>,
}
#[derive(Deserialize)]
struct DraftCard {
    title: String,
    points: Vec<String>,
}

/// Splits raw text into model-sized chunks on paragraph boundaries.
/// ~1400 chars keeps prompt+output well inside the 4096 local context.
pub fn chunk_text(text: &str, target: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if !cur.is_empty() && cur.len() + para.len() + 2 > target {
            chunks.push(std::mem::take(&mut cur));
        }
        // A single paragraph longer than the target is split hard.
        if para.len() > target {
            for piece in para.as_bytes().chunks(target) {
                chunks.push(String::from_utf8_lossy(piece).into_owned());
            }
            continue;
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

pub struct DraftReport {
    /// Drafts in the standard card-markdown contract, ready for the importer.
    pub markdown: String,
    pub chunks: usize,
    pub cards: usize,
    pub warnings: Vec<String>,
}

/// Runs every chunk through the provider and collects draft-card markdown.
/// `on_progress(done, total)` fires after each chunk (drives the UI bar).
pub async fn generate_drafts(
    choice: &ProviderChoice,
    text: &str,
    style: &str,
    on_progress: impl Fn(usize, usize),
) -> Result<DraftReport, String> {
    let chunks = chunk_text(text, 1400);
    if chunks.is_empty() {
        return Err("no material to work with — the text is empty".into());
    }
    let _ = style; // canonical is always default-style; display setting is orthogonal
    let total = chunks.len();
    let sys = system_prompt();

    let mut markdown = String::new();
    let mut cards = 0usize;
    let mut warnings = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        let user = format!("MATERIAL (part {} of {total}):\n{chunk}", i + 1);
        match mode2::complete(choice, sys.clone(), user, 600).await {
            Ok(out) => match parse_drafts(&out) {
                Some(parsed) if !parsed.cards.is_empty() => {
                    for c in parsed.cards {
                        let title = c.title.trim();
                        // Small models repeat bullets and write prose. Tighten
                        // each to the Recommended keyword style, then dedup on
                        // the tightened form so near-duplicates collapse too.
                        let mut seen = std::collections::HashSet::new();
                        let points: Vec<String> = c
                            .points
                            .iter()
                            .map(|p| tighten_default(p.trim().trim_start_matches("- ")))
                            .filter(|p| !p.is_empty() && seen.insert(p.to_lowercase()))
                            .take(crate::cards::MAX_BULLETS)
                            .collect();
                        if title.is_empty() || points.is_empty() {
                            continue;
                        }
                        markdown.push_str(&format!("## {title}\n\n"));
                        for p in &points {
                            markdown.push_str(&format!("- {p}\n"));
                        }
                        markdown.push('\n');
                        cards += 1;
                    }
                }
                _ => warnings.push(format!("part {}: model returned no usable cards", i + 1)),
            },
            Err(e) => warnings.push(format!("part {}: {e}", i + 1)),
        }
        on_progress(i + 1, total);
    }

    if cards == 0 {
        return Err(format!(
            "no cards could be generated ({} part(s) failed): {}",
            warnings.len(),
            warnings.join("; ")
        ));
    }
    Ok(DraftReport {
        markdown,
        chunks: total,
        cards,
        warnings,
    })
}

fn parse_drafts(raw: &str) -> Option<DraftCards> {
    let json = mode2::local::extract_json(raw)?;
    serde_json::from_str(&json).ok()
}

// ── Length-variant backfill (the adapt job) ─────────────────────────
//
// The bullet-length setting restyles the WHOLE corpus in the moment (owner
// decision). Display reads a stored variant per bullet; this job fills the
// missing variants with the configured engine, one call per card.

#[derive(Deserialize)]
struct VariantRow {
    short: String,
    long: String,
}
#[derive(Deserialize)]
struct VariantCards {
    bullets: Vec<VariantRow>,
}

fn variant_system() -> String {
    "You rewrite cue-card bullets into two extra lengths, keeping the SAME meaning \
     and the same language.\n\
     - short: 1-2 words — the sharpest keyword. NEVER copy the whole bullet.\n\
     - long: 10-16 words — the same fact slightly fuller; keyword style, no sentence endings.\n\
     Example bullet: \"moved 40 services, zero downtime\"\n\
     -> short: \"40 services\"\n\
     -> long: \"moved all 40 services, zero downtime, blue-green rollout, no user impact\"\n\
     Never invent new facts, names or numbers; only compress or expand what is there.\n\
     Return ONLY a JSON object: {\"bullets\": [{\"short\": \"...\", \"long\": \"...\"}]} \
     with EXACTLY one entry per input bullet, in the same order."
        .to_string()
}

/// Enforce the variant contracts in code — small models drift (observed
/// live: "short" echoing the full bullet, "long" turning into prose).
fn sanitize_short(base: &str, s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    let n = s.split_whitespace().count();
    if (1..=2).contains(&n) && !s.eq_ignore_ascii_case(base.trim()) {
        s.to_string()
    } else {
        derive_short(base)
    }
}

fn sanitize_long(base: &str, l: &str) -> String {
    let l = l.trim().trim_end_matches('.');
    let n = l.split_whitespace().count();
    // Must actually be fuller than the base, but never balloon into prose.
    if n <= 18 && n > base.split_whitespace().count() {
        l.to_string()
    } else {
        base.to_string()
    }
}

/// Fills missing short/long variants for every bullet that lacks them.
/// `write` persists one bullet's variants (called under the caller's lock
/// discipline); `on_progress(done_cards, total_cards)` drives the UI.
pub async fn adapt_variants(
    choice: &ProviderChoice,
    work: Vec<crate::store::VariantWorkItem>,
    mut write: impl FnMut(&str, &str, &str) -> Result<(), String>,
    on_progress: impl Fn(usize, usize),
) -> Result<usize, String> {
    let total = work.len();
    let mut adapted = 0usize;
    for (i, (_card_id, title, bullets)) in work.into_iter().enumerate() {
        let user = format!(
            "CARD: {title}\nBULLETS:\n{}",
            bullets
                .iter()
                .map(|(_, t)| format!("- {t}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let model_vars: Vec<VariantRow> = match mode2::complete(choice, variant_system(), user, 500)
            .await
        {
            Ok(out) => mode2::local::extract_json(&out)
                .and_then(|json| serde_json::from_str::<VariantCards>(&json).ok())
                .map(|v| v.bullets)
                .unwrap_or_default(),
            Err(e) => {
                tracing::warn!(card = %title, error = %e, "adapt: generation failed — deriving");
                vec![]
            }
        };
        // Every bullet ALWAYS gets variants: model output where it honours
        // the contract, deterministic derivation where it does not. The job
        // must finish 100% or the style switch never becomes instant.
        for (j, (bullet_id, base)) in bullets.iter().enumerate() {
            let (raw_short, raw_long) = model_vars
                .get(j)
                .map(|v| (v.short.as_str(), v.long.as_str()))
                .unwrap_or(("", ""));
            let short = sanitize_short(base, raw_short);
            let long = sanitize_long(base, raw_long);
            write(bullet_id, &short, &long)?;
            adapted += 1;
        }
        on_progress(i + 1, total);
    }
    Ok(adapted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_packs_paragraphs_and_splits_giants() {
        let text = format!("{}\n\n{}\n\n{}", "a".repeat(600), "b".repeat(600), "c".repeat(3000));
        let chunks = chunk_text(&text, 1400);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.len() <= 1400));
    }

    #[test]
    fn variant_sanitizers_enforce_the_contract() {
        let base = "moved 40 services, zero downtime";
        // Model echoing the bullet or rambling → deterministic derivation.
        assert_eq!(sanitize_short(base, base), "moved 40");
        assert_eq!(sanitize_short(base, "40 services"), "40 services");
        assert_eq!(sanitize_short("Cut infra cost 35 percent", ""), "Cut infra");
        // Long must be fuller than base but never prose-ballooned.
        assert_eq!(sanitize_long(base, "short"), base);
        let good = "moved all 40 services with zero downtime, blue-green rollout";
        assert_eq!(sanitize_long(base, good), good);
        assert_eq!(
            sanitize_long(base, &format!("{} extra words {}", good, "x ".repeat(12))).as_str(),
            base
        );
    }

    #[test]
    fn draft_json_parses_from_noisy_output() {
        let noisy = "sure! {\"cards\":[{\"title\":\"Why X?\",\"points\":[\"a\",\"b\"]}]} done";
        let d = parse_drafts(noisy).unwrap();
        assert_eq!(d.cards.len(), 1);
        assert_eq!(d.cards[0].points.len(), 2);
    }
}
