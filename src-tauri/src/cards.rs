//! Markdown card parser. The contract: a card is `## question-style heading`
//! plus optional `tags:` / `lang:` lines plus up to six `- ` bullets.
//! Anything else is rejected loudly, not fixed silently.

use serde::Serialize;

// Owner decision 2026-07-23: genuinely broad topics may carry 7-8 anchors,
// and the prose warning fires only on REAL prose (>15 words) — the default
// keyword style targets ~10, and warning at 11 was pure noise.
pub const MAX_BULLETS: usize = 8;
const PROSE_WORD_LIMIT: usize = 15;

#[derive(Debug, Serialize)]
pub struct ParsedCard {
    pub title: String,
    pub tags: Option<String>,
    pub lang: String,
    pub bullets: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ParseOutcome {
    pub cards: Vec<ParsedCard>,
    /// Human-readable, card-scoped warnings (prose bullets, missing lang…).
    pub warnings: Vec<String>,
    /// Cards rejected outright (over the bullet limit, empty).
    pub rejected: Vec<String>,
}

pub fn parse_markdown(input: &str, default_lang: &str) -> ParseOutcome {
    let mut out = ParseOutcome {
        cards: vec![],
        warnings: vec![],
        rejected: vec![],
    };

    let mut current: Option<ParsedCard> = None;
    for raw in input.lines() {
        let line = raw.trim();
        if let Some(heading) = line.strip_prefix("## ") {
            finish(&mut current, &mut out);
            current = Some(ParsedCard {
                title: heading.trim().to_string(),
                tags: None,
                lang: default_lang.to_string(),
                bullets: vec![],
            });
        } else if let Some(card) = current.as_mut() {
            if let Some(tags) = line.strip_prefix("tags:") {
                card.tags = Some(tags.trim().to_string());
            } else if let Some(lang) = line.strip_prefix("lang:") {
                card.lang = lang.trim().to_string();
            } else if let Some(bullet) = line.strip_prefix("- ") {
                card.bullets.push(bullet.trim().to_string());
            }
        }
    }
    finish(&mut current, &mut out);
    out
}

fn finish(current: &mut Option<ParsedCard>, out: &mut ParseOutcome) {
    let Some(mut card) = current.take() else { return };
    card.bullets.retain(|b| !b.trim().is_empty());
    if card.bullets.is_empty() {
        out.rejected
            .push(format!("\"{}\": no bullets — not a card", card.title));
        return;
    }
    if card.bullets.len() > MAX_BULLETS {
        out.rejected.push(format!(
            "\"{}\": {} bullets — the limit is {}. Split it into two cards.",
            card.title,
            card.bullets.len(),
            MAX_BULLETS
        ));
        return;
    }
    for b in &card.bullets {
        if is_prose(b) {
            out.warnings.push(format!(
                "\"{}\": bullet \"{}\" reads like prose — you are building a script, \
                 and a script will fail you. Cut it to keywords.",
                card.title, b
            ));
        }
    }
    if card.title.split_whitespace().count() < 2 {
        out.warnings.push(format!(
            "\"{}\": heading is a label, not a question — questions retrieve better.",
            card.title
        ));
    }
    out.cards.push(card);
}

/// A bullet is prose when it is long or reads like a full sentence.
fn is_prose(bullet: &str) -> bool {
    let words = bullet.split_whitespace().count();
    words > PROSE_WORD_LIMIT || (words > 6 && bullet.trim_end().ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
## Why are you leaving your own company?
tags: hr, motivation
lang: en

- Good question, love building
- More depth + focus, strong team

## Too many
- a
- b
- c
- d
- e
- f
- g
- h
- i
"#;

    #[test]
    fn parses_cards_and_rejects_over_limit() {
        let out = parse_markdown(SAMPLE, "en");
        assert_eq!(out.cards.len(), 1);
        assert_eq!(out.cards[0].bullets.len(), 2);
        assert_eq!(out.cards[0].tags.as_deref(), Some("hr, motivation"));
        assert_eq!(out.rejected.len(), 1);
        assert!(out.rejected[0].contains("Split it"));
    }

    #[test]
    fn warns_on_prose() {
        let md = "## Q one\n- I built an internal multi-agent platform with around twenty six agents that handles all of the company operations every single day\n";
        let out = parse_markdown(md, "en");
        assert_eq!(out.cards.len(), 1);
        assert_eq!(out.warnings.len(), 1);
    }
}
