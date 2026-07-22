//! Corpus persistence: cards + bullets + FTS + vectors, in one transaction
//! per import. Vectors and FTS rows are derived data — always rebuilt from
//! the cards, never edited directly. Import is additive by design; removal
//! is per-card delete or wipe.
//!
//! Embedding (slow, model work) is separated from writing (fast, needs the
//! DB lock) so callers never hold the connection across model inference.

use crate::cards::ParseOutcome;
use crate::embed::{vec_to_blob, Embedder};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub imported: usize,
    pub warnings: Vec<String>,
    pub rejected: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CardRow {
    pub id: String,
    pub title: String,
    pub tags: Option<String>,
    pub language: String,
    pub source: String,
    pub bullets: Vec<String>,
}

pub struct ImportVectors {
    pub cards: Vec<Vec<f32>>,
    pub bullets: Vec<Vec<f32>>,
}

/// Slow half: model inference only. Call WITHOUT holding the DB lock.
pub fn embed_import(embedder: &Embedder, parsed: &ParseOutcome) -> Result<ImportVectors, String> {
    let card_passages: Vec<(String, String)> = parsed
        .cards
        .iter()
        .map(|c| (c.title.clone(), c.bullets.join("; ")))
        .collect();
    let bullet_passages: Vec<(String, String)> = parsed
        .cards
        .iter()
        .flat_map(|c| c.bullets.iter().map(|b| (String::new(), b.clone())))
        .collect();
    let vectors = ImportVectors {
        cards: embedder.embed_passages(&card_passages)?,
        bullets: embedder.embed_passages(&bullet_passages)?,
    };
    let bullet_count: usize = parsed.cards.iter().map(|c| c.bullets.len()).sum();
    if vectors.cards.len() != parsed.cards.len() || vectors.bullets.len() != bullet_count {
        return Err(format!(
            "embedding count mismatch: {}/{} cards, {}/{} bullets",
            vectors.cards.len(),
            parsed.cards.len(),
            vectors.bullets.len(),
            bullet_count
        ));
    }
    Ok(vectors)
}

/// Fast half: one transaction, no model work.
pub fn write_import(
    conn: &mut Connection,
    parsed: ParseOutcome,
    vectors: ImportVectors,
    session_id: Option<&str>,
) -> Result<ImportReport, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut bullet_vec_iter = vectors.bullets.into_iter();
    for (card, card_vec) in parsed.cards.iter().zip(vectors.cards) {
        let card_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO cards (id, session_id, title, tags, language, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'prepared', strftime('%s','now'))",
            params![card_id, session_id, card.title, card.tags, card.lang],
        )
        .map_err(|e| e.to_string())?;

        for (pos, bullet) in card.bullets.iter().enumerate() {
            let bullet_id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO bullets (id, card_id, position, text, provenance)
                 VALUES (?1, ?2, ?3, ?4, 'prepared')",
                params![bullet_id, card_id, pos as i64, bullet],
            )
            .map_err(|e| e.to_string())?;
            let bv = bullet_vec_iter
                .next()
                .ok_or_else(|| "embedding count mismatch mid-write".to_string())?;
            tx.execute(
                "INSERT INTO bullet_vec (bullet_id, embedding) VALUES (?1, ?2)",
                params![bullet_id, vec_to_blob(&bv)],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.execute(
            "INSERT INTO card_fts (card_id, title, bullets_text) VALUES (?1, ?2, ?3)",
            params![card_id, card.title, card.bullets.join(" ")],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO card_vec (card_id, embedding) VALUES (?1, ?2)",
            params![card_id, vec_to_blob(&card_vec)],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;

    Ok(ImportReport {
        imported: parsed.cards.len(),
        warnings: parsed.warnings,
        rejected: parsed.rejected,
    })
}

/// Convenience wrapper for callers that may hold the lock anyway (tests).
pub fn import_cards(
    conn: &mut Connection,
    embedder: &Embedder,
    parsed: ParseOutcome,
    session_id: Option<&str>,
) -> Result<ImportReport, String> {
    let vectors = embed_import(embedder, &parsed)?;
    write_import(conn, parsed, vectors, session_id)
}

const CARD_SELECT: &str =
    "SELECT c.id, c.title, c.tags, c.language, c.source,
            COALESCE((SELECT group_concat(text, '\u{1f}' ORDER BY position)
                      FROM bullets WHERE card_id = c.id), '')
     FROM cards c";

fn row_to_card(r: &rusqlite::Row<'_>) -> rusqlite::Result<CardRow> {
    let bullets_joined: String = r.get(5)?;
    Ok(CardRow {
        id: r.get(0)?,
        title: r.get(1)?,
        tags: r.get(2)?,
        language: r.get(3)?,
        source: r.get(4)?,
        bullets: if bullets_joined.is_empty() {
            vec![]
        } else {
            bullets_joined.split('\u{1f}').map(String::from).collect()
        },
    })
}

pub fn list_cards(conn: &Connection) -> Result<Vec<CardRow>, String> {
    let sql = format!("{CARD_SELECT} ORDER BY c.created_at DESC, c.title");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_card).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_card(conn: &Connection, card_id: &str) -> Result<Option<CardRow>, String> {
    let sql = format!("{CARD_SELECT} WHERE c.id = ?1");
    conn.query_row(&sql, params![card_id], row_to_card)
        .optional()
        .map_err(|e| e.to_string())
}

/// Bullet embeddings of one card, in display order, decoded to f32.
/// Used by Level-2 coverage: cosine(ME window, each bullet).
pub fn bullet_vectors_for_card(
    conn: &Connection,
    card_id: &str,
) -> Result<Vec<(String, Vec<f32>)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT b.id, v.embedding FROM bullets b
             JOIN bullet_vec v ON v.bullet_id = b.id
             WHERE b.card_id = ?1 ORDER BY b.position",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![card_id], |r| {
            let id: String = r.get(0)?;
            let blob: Vec<u8> = r.get(1)?;
            Ok((id, blob))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, blob) = row.map_err(|e| e.to_string())?;
        let vec = blob
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        out.push((id, vec));
    }
    Ok(out)
}

/// Deletes one card; bullets cascade via FK, derived FTS/vec rows via triggers.
pub fn delete_card(conn: &Connection, card_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM cards WHERE id = ?1", params![card_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn wipe_corpus(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "DELETE FROM bullet_vec; DELETE FROM card_vec; DELETE FROM card_fts;
         DELETE FROM bullets; DELETE FROM cards;",
    )
    .map_err(|e| e.to_string())
}
