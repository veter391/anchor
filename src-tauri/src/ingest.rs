//! Raw-material ingestion (owner decision 2026-07-23): users dump
//! unstructured study text — notes, knowledge bases, whole documents — and
//! the app turns it into draft anchor cards with the configured Mode-2
//! provider (local model by default). Hybrid by design: drafts land in the
//! import box for review/edit, or go straight into the corpus in auto mode.
//! Either way the drafts pass through the SAME markdown contract and parser
//! as hand-written cards — one format, one validation path.

use crate::mode2::provider::style_rule;
use crate::mode2::{self, ProviderChoice};
use serde::Deserialize;

fn system_prompt(style: &str) -> String {
    format!(
        "You turn a person's own study material into cue cards for speaking under pressure.\n\
         A card = one question-style title (phrased the way a person would ASK it in a call) \
         plus 3-6 anchor bullets. The title must accurately describe what its bullets say — \
         never attach one thing's title to another thing's facts.\n\
         {style}\n\
         Use ONLY facts present in the material — never invent names, numbers or claims. \
         Skip filler; if a passage holds nothing worth anchoring, produce fewer cards.\n\
         Keep the material's own language for titles and bullets.\n\
         Return ONLY a JSON object: {{\"cards\": [{{\"title\": \"...\", \"points\": [\"...\"]}}]}} \
         with 1-3 cards.",
        style = style_rule(style)
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
    let total = chunks.len();
    let sys = system_prompt(style);

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
                        // Small models occasionally repeat a bullet — dedup.
                        let mut seen = std::collections::HashSet::new();
                        let points: Vec<&str> = c
                            .points
                            .iter()
                            .map(|p| p.trim().trim_start_matches("- "))
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
    fn draft_json_parses_from_noisy_output() {
        let noisy = "sure! {\"cards\":[{\"title\":\"Why X?\",\"points\":[\"a\",\"b\"]}]} done";
        let d = parse_drafts(noisy).unwrap();
        assert_eq!(d.cards.len(), 1);
        assert_eq!(d.cards[0].points.len(), 2);
    }
}
