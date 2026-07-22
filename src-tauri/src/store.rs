//! Corpus persistence: cards + bullets + FTS + vectors, in one transaction
//! per import. Vectors and FTS rows are derived data — always rebuilt from
//! the cards, never edited directly.

use crate::cards::ParseOutcome;
use crate::embed::{vec_to_blob, Embedder};
use rusqlite::{params, Connection};
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

pub fn import_cards(
    conn: &mut Connection,
    embedder: &Embedder,
    parsed: ParseOutcome,
    session_id: Option<&str>,
) -> Result<ImportReport, String> {
    // Embed outside the transaction: model work is the slow part.
    let card_passages: Vec<(String, String)> = parsed
        .cards
        .iter()
        .map(|c| (c.title.clone(), c.bullets.join("; ")))
        .collect();
    let card_vecs = embedder.embed_passages(&card_passages)?;

    let bullet_passages: Vec<(String, String)> = parsed
        .cards
        .iter()
        .flat_map(|c| c.bullets.iter().map(|b| (String::new(), b.clone())))
        .collect();
    let bullet_vecs = embedder.embed_passages(&bullet_passages)?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut bullet_vec_iter = bullet_vecs.into_iter();
    for (card, card_vec) in parsed.cards.iter().zip(card_vecs) {
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
            let bv = bullet_vec_iter.next().expect("one vector per bullet");
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

pub fn list_cards(conn: &Connection) -> Result<Vec<CardRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.title, c.tags, c.language, c.source,
                    COALESCE((SELECT group_concat(text, '\u{1f}')
                              FROM (SELECT text FROM bullets
                                    WHERE card_id = c.id ORDER BY position)), '')
             FROM cards c
             ORDER BY c.created_at DESC, c.title",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
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
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_card(conn: &Connection, card_id: &str) -> Result<Option<CardRow>, String> {
    Ok(list_cards(conn)?.into_iter().find(|c| c.id == card_id))
}

pub fn wipe_corpus(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "DELETE FROM bullet_vec; DELETE FROM card_vec; DELETE FROM card_fts;
         DELETE FROM bullets; DELETE FROM cards;",
    )
    .map_err(|e| e.to_string())
}
