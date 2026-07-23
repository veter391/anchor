//! Pre-flight research (00_PRODUCT, 03_STACK): paste a URL for the company or
//! role, Anchor fetches the page, strips it to text, and summarises it into a
//! session-scoped CONTEXT card — so "what do you know about us?" is prepared,
//! not a scramble. One fetch, one summary, cached in the session. No crawler.

use crate::embed::Embedder;
use crate::{cards, live, store, Db};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::Manager;

#[derive(Deserialize)]
struct ContextSummary {
    title: String,
    bullets: Vec<String>,
}

#[derive(Serialize)]
pub struct PreflightReport {
    pub title: String,
    pub bullets: usize,
}

/// Case-insensitively remove `<tag ...> ... </tag>` blocks (script/style/etc).
/// `to_ascii_lowercase` preserves byte offsets, so indices are valid on `doc`.
/// Boundary-aware: `<head` must NOT match `<header` (that bug once ate whole
/// pages — the body's first `<header>` had no `</head>` and truncated the rest).
fn remove_blocks(doc: &str, tag: &str) -> String {
    let lower = doc.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(doc.len());
    let mut cursor = 0usize;
    'scan: loop {
        // Find the next REAL opening tag (name followed by a boundary char).
        let mut search = cursor;
        let start = loop {
            match lower[search..].find(&open) {
                Some(rel) => {
                    let s = search + rel;
                    let boundary = lower[s + open.len()..]
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_ascii_alphanumeric());
                    if boundary {
                        break s;
                    }
                    search = s + open.len(); // false prefix match — keep looking
                }
                None => {
                    out.push_str(&doc[cursor..]);
                    break 'scan;
                }
            }
        };
        out.push_str(&doc[cursor..start]);
        match lower[start..].find(&close) {
            Some(crel) => cursor = start + crel + close.len(),
            None => break 'scan, // unclosed block → drop to EOF
        }
    }
    out
}

/// Strip HTML to readable text: drop noise blocks, remove tags, decode a few
/// entities, collapse whitespace. Good enough to brief a model; not a full
/// readability port (that is a Phase-8 refinement).
fn strip_html(html: &str) -> String {
    let mut doc = html.to_string();
    for tag in ["script", "style", "noscript", "svg"] {
        doc = remove_blocks(&doc, tag);
    }
    let mut text = String::with_capacity(doc.len());
    let mut in_tag = false;
    for c in doc.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// First balanced-ish `{...}` block (the model's JSON object).
fn extract_json(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end > start {
        Some(raw[start..=end].to_string())
    } else {
        None
    }
}

fn build_card_md(s: &ContextSummary) -> String {
    let mut md = format!("## {}\ntags: context\nlang: en\n\n", s.title.trim());
    for b in &s.bullets {
        let b = b.trim();
        if !b.is_empty() {
            md.push_str("- ");
            md.push_str(b);
            md.push('\n');
        }
    }
    md
}

/// Fetch a URL, summarise it, and store the result as a CONTEXT card in the
/// session (plus the raw text in context_research for the record).
#[tauri::command]
pub async fn preflight_research(
    app: tauri::AppHandle,
    url: String,
    session_id: String,
) -> Result<PreflightReport, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a full link starting with http:// or https://".into());
    }

    // Snapshot the LLM provider under a short lock (generation is long).
    let choice = {
        let db = app.state::<Db>();
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        live::resolve_provider(&app, &conn).ok_or(
            "No engine configured — download a local model or add an API key in Settings.",
        )?
    };

    // Fetch + strip (network happens with no lock held).
    let client = reqwest::Client::builder()
        .user_agent("Anchor/0.6 (pre-flight research)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Could not reach that page: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("That page returned {}.", resp.status()));
    }
    let html = resp.text().await.map_err(|e| e.to_string())?;
    let mut text = strip_html(&html);
    // Keep the prompt inside the local model's context.
    if text.len() > 6000 {
        text.truncate(6000);
    }
    if text.trim().chars().count() < 40 {
        return Err("Could not read enough text from that page.".into());
    }

    // Summarise into a context card.
    let system = "You brief someone right before a call. From the page text, write a tight \
CONTEXT card. Return ONLY a JSON object: {\"title\": \"<company or product name>\", \
\"bullets\": [\"<what they do>\", \"<product / how they make money>\", \"<stack or notable \
facts>\"]}. 3 to 6 bullets, keyword-dense, no full sentences, and never invent facts that \
are not on the page."
        .to_string();
    let user = format!("PAGE ({url}):\n{text}");
    let raw = crate::mode2::complete(&choice, system, user, 320).await?;
    let json = extract_json(&raw).ok_or("The model did not return a context card.")?;
    let summary: ContextSummary =
        serde_json::from_str(&json).map_err(|e| format!("Could not parse the summary: {e}"))?;
    if summary.title.trim().is_empty() || summary.bullets.iter().all(|b| b.trim().is_empty()) {
        return Err("The summary came back empty.".into());
    }

    // Store as a CONTEXT card scoped to the session, + the raw text for the record.
    let md = build_card_md(&summary);
    let parsed = cards::parse_markdown(&md, "en");
    let embedder = app.state::<Arc<Embedder>>().inner().clone();
    let vectors = store::embed_import(&embedder, &parsed)?;
    {
        let db = app.state::<Db>();
        let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
        store::write_import(&mut conn, parsed, vectors, Some(&session_id), "context")?;
        conn.execute(
            "INSERT OR REPLACE INTO context_research (session_id, raw_text, summary, fetched_at)
             VALUES (?1, ?2, ?3, strftime('%s','now'))",
            rusqlite::params![session_id, text, md],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(PreflightReport {
        title: summary.title.trim().to_string(),
        bullets: summary.bullets.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_in_body_does_not_eat_the_page() {
        // The regression: `<head` prefix-matched `<header>` and truncated the body.
        let html = "<html><head><title>T</title></head><body>\
            <header>site nav</header>\
            <p>Real content about the company and its product stack.</p></body></html>";
        let text = strip_html(html);
        assert!(text.contains("Real content about the company"), "body survived: {text:?}");
    }

    #[test]
    fn scripts_and_styles_are_dropped() {
        let html = "<style>.a{color:red}</style><p>Hello world</p><script>var x=1;</script>";
        assert_eq!(strip_html(html), "Hello world");
    }
}
