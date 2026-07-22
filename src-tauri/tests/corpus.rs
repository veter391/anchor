//! End-to-end corpus pipeline test: parse → import (embeddings, FTS, vectors)
//! → hybrid query, including the cross-lingual case (EN cards, RU query).
//! Requires the embedding model in the local fastembed cache (first run
//! downloads it); this is a real-model test by design.

use anchor::cards::parse_markdown;
use anchor::db;
use anchor::embed::{Embedder, DIMS, MODEL_ID};
use anchor::search::query_cards;
use anchor::store::{import_cards, list_cards};

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
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<*const (), ExtInit>(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
    let dir = std::env::temp_dir().join(format!("anchor-test-{}", std::process::id()));
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
