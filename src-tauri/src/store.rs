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

/// Fast half: one transaction, no model work. `source` is the card provenance
/// tier ("prepared" for user cards; "context" for pre-flight research cards).
pub fn write_import(
    conn: &mut Connection,
    parsed: ParseOutcome,
    vectors: ImportVectors,
    session_id: Option<&str>,
    source: &str,
) -> Result<ImportReport, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut bullet_vec_iter = vectors.bullets.into_iter();
    for (card, card_vec) in parsed.cards.iter().zip(vectors.cards) {
        let card_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO cards (id, session_id, title, tags, language, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%s','now'))",
            params![card_id, session_id, card.title, card.tags, card.lang, source],
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
    write_import(conn, parsed, vectors, session_id, "prepared")
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

/// The global library = cards not owned by any session (session_id IS NULL).
pub fn list_cards(conn: &Connection) -> Result<Vec<CardRow>, String> {
    let sql = format!(
        "{} WHERE c.session_id IS NULL ORDER BY c.created_at DESC, c.title",
        card_select(&display_style(conn))
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_card).map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// One session's own cards.
pub fn list_session_cards(conn: &Connection, session_id: &str) -> Result<Vec<CardRow>, String> {
    let sql = format!(
        "{} WHERE c.session_id = ?1 ORDER BY c.created_at DESC, c.title",
        card_select(&display_style(conn))
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], row_to_card)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The session's expected speech language (ISO code, e.g. "en"/"de"), used to
/// steer the multilingual ASR. `None` if the session is unknown or stored blank
/// — the caller then falls back to auto-detect.
pub fn session_language(conn: &Connection, session_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT language FROM sessions WHERE id = ?1",
        params![session_id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// One bullet's stored row while copying: (id, position, text, short, long, provenance).
type BulletCopyRow = (String, i64, String, Option<String>, Option<String>, String);

#[derive(Serialize)]
pub struct TranscriptLine {
    pub speaker: String,
    pub ts_ms: i64,
    pub text: String,
}

/// A session's stored transcript (confirmed lines only, in order). Text only —
/// audio is never written to disk (08_LEGAL).
pub fn session_transcript(conn: &Connection, session_id: &str) -> Result<Vec<TranscriptLine>, String> {
    let mut stmt = conn
        .prepare("SELECT speaker, ts_ms, text FROM transcript WHERE session_id = ?1 ORDER BY ts_ms, id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            Ok(TranscriptLine {
                speaker: r.get(0)?,
                ts_ms: r.get(1)?,
                text: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Copies cards (with bullets + variants + provenance) to a new owner: a session
/// (`Some(id)`) when pulling from the library, or the global library (`None`)
/// when promoting a session card. The original is left in place; the stored
/// card/bullet vectors and FTS rows are copied verbatim (no re-embed), so
/// retrieval works on the copy immediately. Returns how many were copied.
pub fn copy_cards(
    conn: &mut Connection,
    card_ids: &[String],
    target: Option<&str>,
) -> Result<usize, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut copied = 0usize;
    for src in card_ids {
        let new_card = Uuid::new_v4().to_string();
        // Card row.
        let n = tx
            .execute(
                "INSERT INTO cards (id, session_id, title, tags, language, source, created_at)
                 SELECT ?1, ?2, title, tags, language, source, strftime('%s','now')
                 FROM cards WHERE id = ?3",
                params![new_card, target, src],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            continue;
        }
        // card_vec + card_fts copies.
        tx.execute(
            "INSERT INTO card_vec (card_id, embedding)
             SELECT ?1, embedding FROM card_vec WHERE card_id = ?2",
            params![new_card, src],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO card_fts (card_id, title, bullets_text)
             SELECT ?1, title, bullets_text FROM card_fts WHERE card_id = ?2",
            params![new_card, src],
        )
        .map_err(|e| e.to_string())?;
        // Bullets + their vectors, keeping order and variants.
        let mut stmt = tx
            .prepare(
                "SELECT id, position, text, text_short, text_long, provenance
                 FROM bullets WHERE card_id = ?1 ORDER BY position",
            )
            .map_err(|e| e.to_string())?;
        let bullets: Vec<BulletCopyRow> = stmt
            .query_map(params![src], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        for (old_bid, pos, text, short, long, prov) in bullets {
            let new_bid = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO bullets (id, card_id, position, text, text_short, text_long, provenance)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![new_bid, new_card, pos, text, short, long, prov],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO bullet_vec (bullet_id, embedding)
                 SELECT ?1, embedding FROM bullet_vec WHERE bullet_id = ?2",
                params![new_bid, old_bid],
            )
            .map_err(|e| e.to_string())?;
        }
        copied += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(copied)
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

/// One card's retighten plan — computed WITHOUT embedding or a write lock.
pub struct RetightenFix {
    pub card_id: String,
    pub title: String,
    /// kept bullets: (bullet_id, new_text, new_long, new_short)
    pub keep: Vec<(String, String, String, String)>,
    /// bullet ids to delete (near-duplicates)
    pub drop_ids: Vec<String>,
}

/// The embeddings a fix needs, computed off-lock and handed to `retighten_apply`.
pub struct RetightenVecs {
    pub card_vec: Vec<f32>,
    pub bullet_vecs: Vec<(String, Vec<f32>)>,
    pub fts_text: String,
}

/// Plans the repair of prepared cards whose canonical bullets drifted into
/// prose (older ingested cards): the tight rewrite, kept `text_long`, and
/// within-card duplicate drops. Pure read + compute — NO embedding, so the
/// caller can drop the DB lock before the expensive embed pass (the live
/// ticker needs the same lock every tick). Empty when the corpus is tight.
pub fn retighten_plan(conn: &Connection) -> Result<Vec<RetightenFix>, String> {
    use crate::textfmt::{derive_short, is_too_long, tighten_default};

    let mut cards: Vec<(String, String)> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, title FROM cards WHERE source = 'prepared' ORDER BY created_at")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        while let Some(Ok(row)) = rows.next() {
            cards.push(row);
        }
    }

    let mut fixes: Vec<RetightenFix> = Vec::new();
    for (card_id, title) in cards.drain(..) {
        let mut rows: Vec<(String, String, Option<String>)> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, text, text_long FROM bullets WHERE card_id = ?1 ORDER BY position",
                )
                .map_err(|e| e.to_string())?;
            let mut it = stmt
                .query_map(params![card_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            while let Some(Ok(row)) = it.next() {
                rows.push(row);
            }
        }
        let needs = rows.iter().any(|(_, t, _)| is_too_long(t));
        let has_dup = {
            let mut seen = std::collections::HashSet::new();
            rows.iter().any(|(_, t, _)| !seen.insert(t.to_lowercase()))
        };
        if !needs && !has_dup {
            continue;
        }

        let mut keep: Vec<(String, String, String, String)> = Vec::new();
        let mut drop_ids: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (bid, text, long) in rows {
            let tight = tighten_default(&text);
            if !seen.insert(tight.to_lowercase()) {
                drop_ids.push(bid);
                continue;
            }
            let new_long = long.unwrap_or_else(|| text.trim().trim_end_matches('.').to_string());
            let new_short = derive_short(&tight);
            keep.push((bid, tight, new_long, new_short));
        }
        fixes.push(RetightenFix { card_id, title, keep, drop_ids });
    }
    Ok(fixes)
}

/// Applies a plan (with its off-lock-computed embeddings) in one transaction.
/// `vecs` is parallel to `fixes`. Re-embeds + rebuilds FTS per touched card.
pub fn retighten_apply(
    conn: &mut Connection,
    fixes: &[RetightenFix],
    vecs: &[RetightenVecs],
) -> Result<usize, String> {
    if fixes.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (f, v) in fixes.iter().zip(vecs) {
        for did in &f.drop_ids {
            // vec row rides the bullet delete via trg_bullets_delete.
            tx.execute("DELETE FROM bullets WHERE id = ?1", params![did])
                .map_err(|e| e.to_string())?;
        }
        for (bid, text, long, short) in &f.keep {
            tx.execute(
                "UPDATE bullets SET text = ?2, text_long = ?3, text_short = ?4 WHERE id = ?1",
                params![bid, text, long, short],
            )
            .map_err(|e| e.to_string())?;
        }
        for (bid, vec) in &v.bullet_vecs {
            tx.execute("DELETE FROM bullet_vec WHERE bullet_id = ?1", params![bid])
                .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO bullet_vec (bullet_id, embedding) VALUES (?1, ?2)",
                params![bid, vec_to_blob(vec)],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM card_vec WHERE card_id = ?1", params![f.card_id])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO card_vec (card_id, embedding) VALUES (?1, ?2)",
            params![f.card_id, vec_to_blob(&v.card_vec)],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM card_fts WHERE card_id = ?1", params![f.card_id])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO card_fts (card_id, title, bullets_text) VALUES (?1, ?2, ?3)",
            params![f.card_id, f.title, v.fts_text],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(fixes.len())
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

/// A session's card vectors (card_id, embedding) for session-scoped live
/// retrieval. A session's working set is small (tens of cards), so the ticker
/// brute-forces L2 over these instead of a filtered KNN — cheaper and exact.
pub fn card_vectors_for_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<(String, Vec<f32>)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT v.card_id, v.embedding FROM card_vec v
             JOIN cards c ON c.id = v.card_id
             WHERE c.session_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, blob) = row.map_err(|e| e.to_string())?;
        out.push((id, crate::embed::blob_to_vec(&blob)));
    }
    Ok(out)
}

// ── Coverage report (Phase 6: "what you failed to say") ─────────────

#[derive(Serialize)]
pub struct BulletCoverage {
    pub text: String,
    pub covered: bool,
}
#[derive(Serialize)]
pub struct CardCoverage {
    pub card_id: String,
    pub title: String,
    /// Whether this card actually surfaced during the call (had a card_event
    /// or a covered bullet). A card that never came up is not a "miss".
    pub came_up: bool,
    pub bullets: Vec<BulletCoverage>,
    pub covered: usize,
    pub total: usize,
}
#[derive(Serialize)]
pub struct CoverageReport {
    pub cards: Vec<CardCoverage>,
    /// Anchors covered / total, counted ONLY over cards that came up — the
    /// honest denominator (topics that never arose are not failures).
    pub covered: usize,
    pub total: usize,
    pub verdict: String, // "green" | "red"
    pub untouched_cards: usize,
}

/// A session went "green" when the user covered this share of the anchors on
/// the cards that actually came up.
pub const COVERAGE_GREEN_FLOOR: f64 = 0.7;

/// Builds the post-call coverage report for a session: per card, which anchors
/// were hit vs missed, and a green/red verdict over the cards that came up.
pub fn coverage_report(conn: &Connection, session_id: &str) -> Result<CoverageReport, String> {
    let covered: std::collections::HashSet<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT bullet_id FROM coverage
                 WHERE session_id = ?1 AND covered = 1 AND bullet_id IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map(params![session_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        ids
    };
    // Cards that surfaced during the call: a jump was logged, or a bullet covered.
    let came_up: std::collections::HashSet<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT card_id FROM card_events WHERE session_id = ?1
                 UNION
                 SELECT card_id FROM coverage WHERE session_id = ?1 AND covered = 1",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map(params![session_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        ids
    };
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.title, b.id, b.text
             FROM cards c JOIN bullets b ON b.card_id = c.id
             WHERE c.session_id = ?1
             ORDER BY c.created_at, c.id, b.position",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, String, String)> = stmt
        .query_map(params![session_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut cards: Vec<CardCoverage> = Vec::new();
    let mut untouched = 0usize;
    for (cid, title, bid, text) in rows {
        if cards.last().map(|c| c.card_id.as_str()) != Some(cid.as_str()) {
            let is_up = came_up.contains(&cid);
            if !is_up {
                untouched += 1;
            }
            cards.push(CardCoverage {
                card_id: cid.clone(),
                title,
                came_up: is_up,
                bullets: Vec::new(),
                covered: 0,
                total: 0,
            });
        }
        let card = cards.last_mut().unwrap();
        let hit = covered.contains(&bid);
        card.bullets.push(BulletCoverage { text, covered: hit });
        card.total += 1;
        if hit {
            card.covered += 1;
        }
    }

    let (mut total, mut cov) = (0usize, 0usize);
    for c in &cards {
        if c.came_up {
            total += c.total;
            cov += c.covered;
        }
    }
    let verdict = if total > 0 && (cov as f64) >= (total as f64) * COVERAGE_GREEN_FLOOR {
        "green"
    } else {
        "red"
    };
    Ok(CoverageReport {
        cards,
        covered: cov,
        total,
        verdict: verdict.to_string(),
        untouched_cards: untouched,
    })
}

/// Deletes one card; bullets cascade via FK, derived FTS/vec rows via triggers.
pub fn delete_card(conn: &Connection, card_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM cards WHERE id = ?1", params![card_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn wipe_corpus(conn: &Connection) -> Result<(), String> {
    // card_events.card_id has no FK to cards (it is a retrieval log, kept even
    // if a card is later edited), so deleting cards does NOT cascade to it —
    // wipe it explicitly or a full corpus wipe leaves dangling match-log rows.
    // coverage and card_vec DO cascade from cards (ON DELETE CASCADE), so the
    // cards delete already clears those.
    conn.execute_batch(
        "DELETE FROM bullet_vec; DELETE FROM card_vec; DELETE FROM card_fts;
         DELETE FROM card_events; DELETE FROM bullets; DELETE FROM cards;",
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_language_reads_trimmed_nonblank_or_none() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, language TEXT NOT NULL);
             INSERT INTO sessions VALUES ('s1','es'), ('s2',''), ('s3','  uk  ');",
        )
        .unwrap();
        assert_eq!(session_language(&conn, "s1").as_deref(), Some("es"));
        assert_eq!(session_language(&conn, "s3").as_deref(), Some("uk")); // trimmed
        assert_eq!(session_language(&conn, "s2"), None); // blank → auto-detect
        assert_eq!(session_language(&conn, "missing"), None); // unknown session
    }
}
