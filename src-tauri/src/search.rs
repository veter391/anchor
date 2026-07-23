//! Hybrid retrieval: vector KNN (sqlite-vec) + BM25 (FTS5), fused with
//! Reciprocal Rank Fusion. The keyword leg is the insurance for proper
//! nouns and numbers; the vector leg absorbs paraphrase and cross-lingual
//! queries. See Documents/02_ARCHITECTURE.md §3.

use crate::embed::{vec_to_blob, Embedder};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

pub const RRF_K: f64 = 60.0;
const LEG_LIMIT: usize = 10;

// RRF fused score orders the candidate list (it fuses the vector and keyword
// legs, catching proper nouns the vector leg misses). It is NOT used for the
// hysteresis thresholds: rank-based scores are near-identical across the top
// cards, so they cannot express "how much closer" one topic is. The match
// engine scores cards by real cosine similarity instead (see live.rs).

#[derive(Debug, Serialize, Clone)]
pub struct Match {
    pub card_id: String,
    pub fused: f64,
    pub vec_rank: Option<usize>,
    pub vec_distance: Option<f64>,
    pub bm25_rank: Option<usize>,
}

/// Embeds the query. Call WITHOUT holding the DB lock (model work is slow).
pub fn embed_query_text(embedder: &Embedder, text: &str) -> Result<Vec<f32>, String> {
    embedder.embed_query(text)
}

pub fn query_cards(
    conn: &Connection,
    embedder: &Embedder,
    text: &str,
) -> Result<Vec<Match>, String> {
    let qvec = embed_query_text(embedder, text)?;
    query_cards_with_vec(conn, &qvec, text)
}

/// Hybrid query with a pre-computed query vector — no model work inside.
pub fn query_cards_with_vec(
    conn: &Connection,
    qvec: &[f32],
    text: &str,
) -> Result<Vec<Match>, String> {
    let mut legs: HashMap<String, Match> = HashMap::new();

    // Vector leg.
    {
        let mut stmt = conn
            .prepare(
                "SELECT card_id, distance FROM card_vec
                 WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![vec_to_blob(qvec), LEG_LIMIT as i64], |r| {
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

    Ok(fuse(legs))
}

/// RRF fusion over the collected legs → ranked matches. Shared by the global
/// and the session-scoped query paths so both fuse identically.
fn fuse(legs: HashMap<String, Match>) -> Vec<Match> {
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
    matches
}

/// L2 (Euclidean) distance — matches sqlite-vec's vec0 default metric, so the
/// ticker's `cos = 1 − d²/2` (valid for unit vectors) stays correct on both paths.
fn l2(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let d = (x - y) as f64;
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

/// Session-scoped hybrid retrieval: the same RRF fusion, but only over cards
/// owned by `session_id`. The vector leg brute-forces L2 over the session's
/// stored card vectors (a session's working set is small — tens of cards — so
/// this is cheaper than a filtered KNN and exact, with no pruning miss); the
/// keyword leg joins card_fts to cards on the session. The global path
/// (`query_cards_with_vec`) is deliberately left untouched.
pub fn query_cards_scoped(
    conn: &Connection,
    qvec: &[f32],
    text: &str,
    session_id: &str,
) -> Result<Vec<Match>, String> {
    let mut legs: HashMap<String, Match> = HashMap::new();

    // Vector leg: brute-force L2 over the session's card vectors.
    let mut scored: Vec<(String, f64)> = crate::store::card_vectors_for_session(conn, session_id)?
        .into_iter()
        .map(|(id, v)| (id, l2(qvec, &v)))
        .collect();
    scored.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (rank, (card_id, dist)) in scored.into_iter().take(LEG_LIMIT).enumerate() {
        let entry = legs.entry(card_id.clone()).or_insert(Match {
            card_id,
            fused: 0.0,
            vec_rank: None,
            vec_distance: None,
            bm25_rank: None,
        });
        entry.vec_rank = Some(rank + 1);
        entry.vec_distance = Some(dist);
    }

    // Keyword leg: FTS5 joined to the session.
    if let Some(fts_query) = build_fts_query(text) {
        let mut stmt = conn
            .prepare(
                "SELECT f.card_id FROM card_fts f JOIN cards c ON c.id = f.card_id
                 WHERE card_fts MATCH ?1 AND c.session_id = ?2 ORDER BY rank LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![fts_query, session_id, LEG_LIMIT as i64], |r| {
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

    Ok(fuse(legs))
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
