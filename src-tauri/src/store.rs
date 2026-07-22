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

/// The active bullet-length mode ("default" | "short" | "long"). Reading it
/// here keeps every card read path — dashboard AND overlay — in the chosen
/// style with zero caller changes (owner decision: switching the setting
/// restyles the whole corpus in the moment).
pub fn display_style(conn: &Connection) -> String {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'bullet_style'",
        [],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| "default".into())
}

/// Bullet text column expression for a style: variants fall back to the
/// canonical text until the adapt job has produced them.
fn bullet_expr(style: &str) -> &'static str {
    match style {
        "short" => "COALESCE(text_short, text)",
        "long" => "COALESCE(text_long, text)",
        _ => "text",
    }
}

fn card_select(style: &str) -> String {
    format!(
        "SELECT c.id, c.title, c.tags, c.language, c.source,
                COALESCE((SELECT group_concat({expr}, '\u{1f}' ORDER BY position)
                          FROM bullets WHERE card_id = c.id), '')
         FROM cards c",
        expr = bullet_expr(style)
    )
}

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
    let sql = format!(
        "{} ORDER BY c.created_at DESC, c.title",
        card_select(&display_style(conn))
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_card).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_card(conn: &Connection, card_id: &str) -> Result<Option<CardRow>, String> {
    let sql = format!("{} WHERE c.id = ?1", card_select(&display_style(conn)));
    conn.query_row(&sql, params![card_id], row_to_card)
        .optional()
        .map_err(|e| e.to_string())
}

/// One adapt-job unit: (card_id, title, [(bullet_id, canonical_text)]).
pub type VariantWorkItem = (String, String, Vec<(String, String)>);

/// Bullets missing a length variant, grouped per card — the adapt job's
/// work list.
pub fn cards_needing_variants(conn: &Connection) -> Result<Vec<VariantWorkItem>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT b.card_id, c.title, b.id, b.text FROM bullets b
             JOIN cards c ON c.id = b.card_id
             WHERE b.text_short IS NULL OR b.text_long IS NULL
             ORDER BY b.card_id, b.position",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out: Vec<VariantWorkItem> = Vec::new();
    for row in rows.flatten() {
        let (card_id, title, bullet_id, text) = row;
        match out.last_mut() {
            Some((cid, _, list)) if *cid == card_id => list.push((bullet_id, text)),
            _ => out.push((card_id, title, vec![(bullet_id, text)])),
        }
    }
    Ok(out)
}

pub fn set_bullet_variants(
    conn: &Connection,
    bullet_id: &str,
    short: &str,
    long: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE bullets SET text_short = ?2, text_long = ?3 WHERE id = ?1",
        params![bullet_id, short, long],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
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
