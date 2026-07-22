//! Hybrid retrieval: vector KNN (sqlite-vec) + BM25 (FTS5), fused with
//! Reciprocal Rank Fusion. The keyword leg is the insurance for proper
//! nouns and numbers; the vector leg absorbs paraphrase and cross-lingual
//! queries. See Documents/02_ARCHITECTURE.md §3.

use crate::embed::{vec_to_blob, Embedder};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

const RRF_K: f64 = 60.0;
const LEG_LIMIT: usize = 10;

#[derive(Debug, Serialize, Clone)]
pub struct Match {
    pub card_id: String,
    pub fused: f64,
    pub vec_rank: Option<usize>,
    pub vec_distance: Option<f64>,
    pub bm25_rank: Option<usize>,
}

pub fn query_cards(
    conn: &Connection,
    embedder: &Embedder,
    text: &str,
) -> Result<Vec<Match>, String> {
    let mut legs: HashMap<String, Match> = HashMap::new();

    // Vector leg.
    let qvec = embedder.embed_query(text)?;
    {
        let mut stmt = conn
            .prepare(
                "SELECT card_id, distance FROM card_vec
                 WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![vec_to_blob(&qvec), LEG_LIMIT as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for (rank, row) in rows.flatten().enumerate() {
            let (card_id, distance) = row;
            let entry = legs.entry(card_id.clone()).or_insert(Match {
                card_id,
                fused: 0.0,
                vec_rank: None,
                vec_distance: None,
                bm25_rank: None,
            });
            entry.vec_rank = Some(rank + 1);
            entry.vec_distance = Some(distance);
        }
    }

    // Keyword leg. Free text → OR-of-tokens FTS query, punctuation stripped.
    if let Some(fts_query) = build_fts_query(text) {
        let mut stmt = conn
            .prepare(
                "SELECT card_id FROM card_fts WHERE card_fts MATCH ?1
                 ORDER BY rank LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![fts_query, LEG_LIMIT as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        for (rank, card_id) in rows.flatten().enumerate() {
            let entry = legs.entry(card_id.clone()).or_insert(Match {
                card_id,
                fused: 0.0,
                vec_rank: None,
                vec_distance: None,
                bm25_rank: None,
            });
            entry.bm25_rank = Some(rank + 1);
        }
    }

    // RRF fusion.
    let mut matches: Vec<Match> = legs
        .into_values()
        .map(|mut m| {
            let v = m.vec_rank.map_or(0.0, |r| 1.0 / (RRF_K + r as f64));
            let b = m.bm25_rank.map_or(0.0, |r| 1.0 / (RRF_K + r as f64));
            m.fused = v + b;
            m
        })
        .collect();
    matches.sort_by(|a, b| b.fused.total_cmp(&a.fused));
    matches.truncate(LEG_LIMIT);
    Ok(matches)
}

/// Lowercase, strip punctuation, quote each token, OR them together.
/// Quoting keeps FTS5 from parsing tokens as query syntax.
fn build_fts_query(text: &str) -> Option<String> {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .map(|t| format!("\"{}\"", t.to_lowercase()))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}
