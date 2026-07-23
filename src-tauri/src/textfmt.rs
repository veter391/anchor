//! Deterministic bullet-style normalizers. The product's whole premise is
//! "keywords, not sentences — a script will fail you" (00_PRODUCT). Small
//! local models write full sentences anyway, so we enforce the Recommended
//! (default) keyword-dense style in code, exactly like the short/long variant
//! sanitizers. Idempotent on already-tight bullets (hand-written cards pass
//! through untouched).

/// The Recommended-style budget. At or under this, a bullet is already tight.
pub const DEFAULT_MAX_WORDS: usize = 10;

/// Pure grammatical glue that carries no anchor value. Kept deliberately
/// small — over-stripping mangles meaning; we only drop the safest fillers.
const FILLER: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "of", "to", "with", "and",
    "that", "which", "it", "its", "their", "them", "as", "so", "then", "into", "onto", "your",
    "you", "we", "our", "will", "would", "can", "could",
];

/// Words we never drop regardless of the filler list: proper nouns / acronyms
/// (any uppercase), anything with a digit, and symbol-bearing anchors (+ % & /).
fn is_significant(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        || word.contains(['+', '%', '&', '/'])
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Tightens a bullet to the Recommended keyword style: drop articles/copulas/
/// filler, keep proper nouns, numbers, symbols and commas, cap the length.
/// Returns already-tight input unchanged.
pub fn tighten_default(text: &str) -> String {
    let text = text.trim().trim_end_matches(['.', ';']);
    if word_count(text) <= DEFAULT_MAX_WORDS {
        return text.to_string();
    }
    let mut out: Vec<String> = Vec::new();
    for (i, raw) in text.split_whitespace().enumerate() {
        let trailing_comma = raw.ends_with(',');
        let core = raw.trim_end_matches(',');
        let key = core.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        let keep = i == 0 || is_significant(core) || !FILLER.contains(&key.as_str());
        if keep {
            out.push(if trailing_comma {
                format!("{core},")
            } else {
                core.to_string()
            });
            if out.len() >= DEFAULT_MAX_WORDS {
                break;
            }
        }
    }
    let s = out.join(" ");
    let s = s.trim_end_matches(',').to_string();
    if s.is_empty() {
        text.to_string()
    } else {
        s
    }
}

/// A 1-2 word keyword anchor — the deterministic Short variant / fallback.
pub fn derive_short(base: &str) -> String {
    let words: Vec<&str> = base
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '+' || c == '%'))
        .filter(|w| {
            w.chars().any(char::is_alphanumeric)
                && !FILLER.contains(&w.to_lowercase().as_str())
        })
        .take(2)
        .collect();
    if words.is_empty() {
        base.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
    } else {
        words.join(" ")
    }
}

/// Enforces the requested display style on one bullet (used for live Mode-2
/// cards, which are not stored and so have no variant columns).
pub fn enforce_style(text: &str, style: &str) -> String {
    match style {
        "short" => derive_short(text),
        // "long" keeps whatever the model produced; only guard against prose.
        "long" => text.trim().trim_end_matches('.').to_string(),
        _ => tighten_default(text),
    }
}

/// True when a bullet is too long for the Recommended style (repair trigger).
pub fn is_too_long(text: &str) -> bool {
    word_count(text.trim()) > DEFAULT_MAX_WORDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tight_bullets_pass_through_unchanged() {
        for b in [
            "40 services, zero-downtime",
            "Helm + ArgoCD, GitOps",
            "Good question, love building",
            "Total comp, not base only",
        ] {
            assert_eq!(tighten_default(b), b, "should not touch an already-tight bullet");
        }
    }

    #[test]
    fn prose_is_tightened_to_keywords_within_budget() {
        let cases = [
            "RabbitMQ is a smart broker with routing, acknowledgements and per-message delivery guarantees",
            "GraphQL exposes a single endpoint and lets the client describe exactly the data it needs",
            "Consumers keep their own offsets, which makes replay and multiple independent readers cheap",
        ];
        for c in cases {
            let t = tighten_default(c);
            assert!(word_count(&t) <= DEFAULT_MAX_WORDS, "over budget: {t}");
            assert!(!t.is_empty());
        }
        // Proper nouns and numbers survive; articles/copulas are gone.
        let t = tighten_default(
            "RabbitMQ is a smart broker with routing, acknowledgements and per-message delivery guarantees",
        );
        assert!(t.starts_with("RabbitMQ"));
        assert!(!t.split_whitespace().any(|w| w == "is" || w == "a" || w == "and"));
    }

    #[test]
    fn short_is_one_or_two_keywords() {
        assert_eq!(derive_short("moved 40 services, zero downtime"), "moved 40");
        let s = derive_short("RabbitMQ is a smart broker");
        assert!(s.split_whitespace().count() <= 2 && s.starts_with("RabbitMQ"));
    }
}
