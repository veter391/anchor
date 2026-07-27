//! End-to-end corpus pipeline test: parse → import (embeddings, FTS, vectors)
//! → hybrid query, including the cross-lingual case (EN cards, RU query).
//! Requires the embedding model in the local fastembed cache (first run
//! downloads it); this is a real-model test by design.

use anchor::cards::parse_markdown;
use anchor::db;
use anchor::embed::{Embedder, DIMS, MODEL_ID};
use anchor::search::{embed_query_text, query_cards, query_cards_scoped};
use anchor::store::{coverage_report, get_card, import_cards, list_cards, list_session_cards};
use std::collections::HashSet;

const CORPUS: &str = r#"
## Why are you leaving your own company?
tags: hr, motivation
lang: en

- Good question, love building
- More depth + focus, strong team
- Project long-term, automated
- Learn from seniors, grow

## Tell me about the Kubernetes migration
tags: tech
lang: en

- 40 services, zero-downtime
- Helm + ArgoCD, GitOps
- Cut infra cost 35 percent

## What are your salary expectations?
tags: hr
lang: en

- Range researched, market-based
- Flexible for the right role
- Total comp, not base only
"#;

fn setup() -> (rusqlite::Connection, Embedder) {
    type ExtInit = unsafe extern "C" fn(
        *mut rusqlite::ffi::sqlite3,
        *mut *mut std::os::raw::c_char,
        *const rusqlite::ffi::sqlite3_api_routines,
    ) -> std::os::raw::c_int;
    // Register the vec extension once — it is process-global; racing it across
    // parallel tests is unsafe.
    static VEC_ONCE: std::sync::Once = std::sync::Once::new();
    VEC_ONCE.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<*const (), ExtInit>(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
    // A unique dir per setup() call so parallel tests never share a DB file
    // (otherwise the second open hits "database is locked").
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("anchor-test-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    let conn = db::open_and_migrate(&dir.join("test.db")).unwrap();
    let (_, dims) = db::ensure_embedding_config(&conn, MODEL_ID, DIMS as i64).unwrap();
    db::ensure_vec_tables(&conn, dims).unwrap();
    (conn, Embedder::new())
}

#[test]
fn import_then_hybrid_query_including_cross_lingual() {
    let (mut conn, embedder) = setup();

    let parsed = parse_markdown(CORPUS, "en");
    assert_eq!(parsed.cards.len(), 3, "all three cards parse");
    let report = import_cards(&mut conn, &embedder, parsed, None).unwrap();
    assert_eq!(report.imported, 3);
    let listed = list_cards(&conn).unwrap();
    assert_eq!(listed.len(), 3);
    let leaving = listed
        .iter()
        .find(|c| c.title.starts_with("Why are you leaving"))
        .unwrap();
    assert_eq!(
        leaving.bullets,
        vec![
            "Good question, love building",
            "More depth + focus, strong team",
            "Project long-term, automated",
            "Learn from seniors, grow"
        ],
        "bullets keep display order"
    );

    let title_of = |id: &str| {
        list_cards(&conn)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
            .title
    };

    // English paraphrase → semantic leg must carry it.
    let m = query_cards(&conn, &embedder, "so why do you want to move on from the business you founded").unwrap();
    assert!(!m.is_empty());
    assert_eq!(
        title_of(&m[0].card_id),
        "Why are you leaving your own company?",
        "EN paraphrase retrieves the right card"
    );

    // Proper noun → keyword leg insurance.
    let m = query_cards(&conn, &embedder, "how did the ArgoCD rollout go").unwrap();
    assert_eq!(
        title_of(&m[0].card_id),
        "Tell me about the Kubernetes migration",
        "proper noun hits via BM25 leg"
    );
    assert!(m[0].bm25_rank.is_some(), "BM25 leg participated");

    // Cross-lingual: Russian question, English cards.
    let m = query_cards(&conn, &embedder, "почему ты уходишь из своей собственной компании").unwrap();
    assert_eq!(
        title_of(&m[0].card_id),
        "Why are you leaving your own company?",
        "RU query retrieves the EN card (cross-lingual embedding space)"
    );

    // Spanish, second topic.
    let m = query_cards(&conn, &embedder, "cuánto esperas ganar, expectativas de salario").unwrap();
    assert_eq!(
        title_of(&m[0].card_id),
        "What are your salary expectations?",
        "ES query retrieves the EN salary card"
    );
}

const S1_CARDS: &str = r#"
## Tell me about the Kubernetes migration
lang: en

- 40 services, zero-downtime
- Helm + ArgoCD, GitOps

## Why are you leaving your own company?
lang: en

- Good question, love building
- More depth + focus
"#;

const S2_CARDS: &str = r#"
## What are your salary expectations?
lang: en

- Range researched, market-based
- Flexible for the right role
"#;

/// The live loop, when bound to a session, must retrieve ONLY that session's
/// own cards — a card living in another session (or the global library) must
/// never surface. This is the load-bearing guarantee of the Phase-6 session
/// loop (05_DATA_MODEL: "sessions import from it"; 02_ARCHITECTURE §5).
#[test]
fn session_scoped_query_only_returns_session_cards() {
    let (mut conn, embedder) = setup();
    for (id, title) in [("s1", "Session One"), ("s2", "Session Two")] {
        conn.execute(
            "INSERT INTO sessions (id, title, kind, status, language, created_at)
             VALUES (?1, ?2, 'other', 'planned', 'en', 0)",
            rusqlite::params![id, title],
        )
        .unwrap();
    }
    import_cards(&mut conn, &embedder, parse_markdown(S1_CARDS, "en"), Some("s1")).unwrap();
    import_cards(&mut conn, &embedder, parse_markdown(S2_CARDS, "en"), Some("s2")).unwrap();

    let s1_ids: HashSet<String> = list_session_cards(&conn, "s1")
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(s1_ids.len(), 2, "s1 owns exactly its two cards");

    // A salary question against s1 must NOT surface s2's salary card — the
    // scope holds even though salary is the strongest match globally.
    let salary_q = "what are your salary expectations";
    let qv = embed_query_text(&embedder, salary_q).unwrap();
    let scoped = query_cards_scoped(&conn, &qv, salary_q, "s1").unwrap();
    assert!(!scoped.is_empty(), "s1 still returns its own cards");
    for m in &scoped {
        assert!(
            s1_ids.contains(&m.card_id),
            "session scope leaked: a non-s1 card surfaced"
        );
    }

    // Within s1, the kubernetes question ranks the kubernetes card first
    // (semantic + BM25 both live on the scoped path).
    let kube_q = "how did the kubernetes migration go for your team";
    let kv = embed_query_text(&embedder, kube_q).unwrap();
    let mk = query_cards_scoped(&conn, &kv, kube_q, "s1").unwrap();
    let top = get_card(&conn, &mk[0].card_id).unwrap().unwrap();
    assert!(
        top.title.contains("Kubernetes"),
        "kubernetes card ranks first within s1, got {:?}",
        top.title
    );

    // s2 surfaces its own salary card for the same salary question.
    let ms = query_cards_scoped(&conn, &qv, salary_q, "s2").unwrap();
    let s2top = get_card(&conn, &ms[0].card_id).unwrap().unwrap();
    assert!(
        s2top.title.contains("salary"),
        "s2 surfaces its salary card, got {:?}",
        s2top.title
    );
}

/// The coverage report is the Phase-6 DoD: it counts only the cards that came
/// up during the call, marks each anchor hit/missed, and scores green/red.
#[test]
fn coverage_report_counts_only_cards_that_came_up() {
    let (mut conn, embedder) = setup();
    conn.execute(
        "INSERT INTO sessions (id, title, kind, status, language, created_at)
         VALUES ('r1', 'Report', 'other', 'planned', 'en', 0)",
        [],
    )
    .unwrap();
    import_cards(&mut conn, &embedder, parse_markdown(S1_CARDS, "en"), Some("r1")).unwrap();
    // S1_CARDS = kubernetes (2 bullets) + leaving (2 bullets), both owned by r1.
    let cards = list_session_cards(&conn, "r1").unwrap();
    let kube = cards.iter().find(|c| c.title.contains("Kubernetes")).unwrap();
    let bullet_ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM bullets WHERE card_id = ?1 ORDER BY position")
            .unwrap();
        stmt.query_map(rusqlite::params![kube.id], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    assert_eq!(bullet_ids.len(), 2);

    // Simulate a call: only the kubernetes card came up; 1 of its 2 anchors hit.
    conn.execute(
        "INSERT INTO card_events (session_id, card_id, ts_ms, mode)
         VALUES ('r1', ?1, 0, 'retrieved')",
        rusqlite::params![kube.id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO coverage (session_id, card_id, bullet_id, covered)
         VALUES ('r1', ?1, ?2, 1)",
        rusqlite::params![kube.id, bullet_ids[0]],
    )
    .unwrap();

    let rep = coverage_report(&conn, "r1").unwrap();
    let up: Vec<_> = rep.cards.iter().filter(|c| c.came_up).collect();
    assert_eq!(up.len(), 1, "only the kubernetes card came up");
    assert!(up[0].title.contains("Kubernetes"));
    assert_eq!(rep.total, 2, "denominator is only the came-up card's anchors");
    assert_eq!(rep.covered, 1);
    assert_eq!(rep.untouched_cards, 1, "the leaving card never came up");
    assert_eq!(rep.verdict, "red", "1 of 2 = 50% < 70% floor");

    // Cover the second anchor → green.
    conn.execute(
        "INSERT INTO coverage (session_id, card_id, bullet_id, covered)
         VALUES ('r1', ?1, ?2, 1)",
        rusqlite::params![kube.id, bullet_ids[1]],
    )
    .unwrap();
    let rep2 = coverage_report(&conn, "r1").unwrap();
    assert_eq!(rep2.covered, 2);
    assert_eq!(rep2.verdict, "green", "2 of 2 = 100% >= 70% floor");
}

#[test]
fn real_asr_transcripts_of_generated_es_ru_uk_voices_hit_the_right_en_card() {
    // These are the ACTUAL output of the shipped multilingual ASR on ElevenLabs-
    // generated ES/RU/UK interviewer questions (Phase-7.4, 2026-07-27 — the voices
    // live in spike/voices, the transcripts came from the `asr_file` example).
    // Proves the full launch-language path end to end: generated speech → ASR →
    // cross-lingual retrieval → the correct EN card, for ALL launch languages
    // (Ukrainian included, which the older test above did not cover).
    let (mut conn, embedder) = setup();
    let parsed = parse_markdown(CORPUS, "en");
    import_cards(&mut conn, &embedder, parsed, None).unwrap();
    let title_of = |id: &str| {
        list_cards(&conn)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
            .title
    };

    let cases = [
        ("ES", "dejas tu propia empresa para asumir un nuevo", "Why are you leaving your own company?"),
        ("RU", "Расскажите, как прошла миграция на кубернете сыйшей комане", "Tell me about the Kubernetes migration"),
        ("UK", "Яки у вас очікування щодо зарплати най посаді", "What are your salary expectations?"),
    ];
    for (lang, transcript, expected) in cases {
        let m = query_cards(&conn, &embedder, transcript).unwrap();
        assert!(!m.is_empty(), "{lang}: retrieval returned a candidate");
        assert_eq!(
            title_of(&m[0].card_id),
            expected,
            "{lang} ASR transcript retrieves the right EN card (cross-lingual)"
        );
    }
}
