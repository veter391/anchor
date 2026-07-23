//! Scale-truth bench for the corpus pipeline (owner asked: "what happens when
//! we load a LOT of material — import speed, embeddings, retrieval, memory?").
//!
//! Measures the REAL, shipped path on this machine: embed a large synthetic
//! corpus (EmbeddingGemma-256d via fastembed), write it into SQLite +
//! sqlite-vec + FTS5, then run many hybrid retrieval queries.
//!
//! Run:  build-env.bat cargo run --release --example scale_bench --no-default-features
//! Numbers land in 10_RESEARCH_LOG. This is measurement, not product code.

use anchor::{cards, db, embed, embed::Embedder, search, store};
use std::time::Instant;

// Distinct topic templates so titles/bullets carry real, varied vocabulary
// (otherwise FTS + vectors would see near-duplicates and flatter the numbers).
const TOPICS: &[(&str, &[&str])] = &[
    ("leaving your current company", &["love building things", "want more depth, strong team", "long-term project, automated", "focus energy into this role", "learn from seniors, grow"]),
    ("the Kubernetes migration", &["moved 40 services, zero downtime", "Helm + ArgoCD, full GitOps", "cut infra cost 35 percent", "blue-green rollout, no user impact", "observability with Prometheus"]),
    ("salary expectations", &["researched market range", "flexible for the right role", "total comp, not base only", "value growth over cash", "open to equity discussion"]),
    ("a system design tradeoff", &["CAP: chose availability", "eventual consistency, CRDTs", "sharded by tenant id", "read replicas, write leader", "cache-aside with Redis"]),
    ("handling production incidents", &["blameless postmortem culture", "runbooks, on-call rotation", "MTTR under 30 minutes", "feature flags to roll back", "SLO error budget policy"]),
    ("mentoring junior engineers", &["pair on hard reviews", "unblock, do not solve for them", "small tickets first, grow scope", "weekly 1:1, real feedback", "celebrate their wins publicly"]),
    ("a database performance problem", &["N+1 query, added eager load", "composite index on hot path", "connection pool exhaustion", "moved report to read replica", "explain analyze before guessing"]),
    ("why microservices here", &["independent deploy cadence", "team ownership boundaries", "blast radius isolation", "polyglot where it pays", "not for a small team"]),
    ("a conflict on the team", &["listened, restated their point", "data over opinion", "disagree and commit", "escalated with options, not drama", "followed up, repaired trust"]),
    ("your testing philosophy", &["fast unit core, few e2e", "test behaviour, not internals", "flaky test is a bug", "coverage is a signal, not a goal", "TDD on gnarly logic"]),
    ("scaling the ingestion pipeline", &["backpressure, bounded queues", "idempotent consumers", "batch writes, amortize IO", "dead-letter for poison rows", "measure p95, not average"]),
    ("a security review finding", &["injection: parametrized queries", "secrets in the keychain", "least-privilege IAM", "signed, verified downloads", "threat model before code"]),
];

fn gen_markdown(n: usize) -> String {
    let mut s = String::with_capacity(n * 240);
    for i in 0..n {
        let (q, bullets) = TOPICS[i % TOPICS.len()];
        // vary the phrasing per instance so cards are not literal duplicates
        s.push_str(&format!("## Case {i}: tell me about {q}\nlang: en\n\n"));
        for b in bullets {
            s.push_str(&format!("- {b}\n"));
        }
        s.push('\n');
    }
    s
}

fn pct(sorted_ms: &[f64], p: f64) -> f64 {
    if sorted_ms.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_ms.len() as f64 - 1.0) * p).round() as usize;
    sorted_ms[idx]
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sizes = if sizes.is_empty() { vec![250, 1000, 2000] } else { sizes };

    println!("== Anchor corpus scale bench (EmbeddingGemma-{}d) ==", embed::DIMS);
    db::register_vec_extension();
    let embedder = Embedder::new();
    let t = Instant::now();
    let _ = embedder.embed_query("warmup pass to load the model").unwrap();
    println!("model warm-up (first embed, incl. load): {:.0} ms\n", t.elapsed().as_secs_f64() * 1000.0);

    let queries = [
        "so why are you thinking about leaving your job",
        "how did that kubernetes thing go for you",
        "what kind of money are you looking for",
        "walk me through a tough architecture decision",
        "tell me about a bad outage you handled",
        "how do you grow the juniors on your team",
        "what do you like to do outside of work for fun",
    ];

    for &n in &sizes {
        let md = gen_markdown(n);
        let parsed = cards::parse_markdown(&md, "en");
        let n_cards = parsed.cards.len();
        let n_bullets: usize = parsed.cards.iter().map(|c| c.bullets.len()).sum();
        let n_embeds = n_cards + n_bullets;

        let path = std::env::temp_dir().join(format!("anchor_bench_{n}.db"));
        let _ = std::fs::remove_file(&path);
        let mut conn = db::open_and_migrate(&path).unwrap();
        let (_, dims) =
            db::ensure_embedding_config(&conn, embed::MODEL_ID, embed::DIMS as i64).unwrap();
        db::ensure_vec_tables(&conn, dims).unwrap();

        let t = Instant::now();
        let vecs = store::embed_import(&embedder, &parsed).unwrap();
        let embed_ms = t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        let _ = store::write_import(&mut conn, parsed, vecs, None, "prepared").unwrap();
        let write_ms = t.elapsed().as_secs_f64() * 1000.0;

        // Retrieval: separate the embed leg (per tick) from the pure DB query.
        let mut embed_q = Vec::new();
        let mut db_q = Vec::new();
        let rounds = 210;
        for i in 0..rounds {
            let q = queries[i % queries.len()];
            let t = Instant::now();
            let qv = search::embed_query_text(&embedder, q).unwrap();
            embed_q.push(t.elapsed().as_secs_f64() * 1000.0);
            let t = Instant::now();
            let _ = search::query_cards_with_vec(&conn, &qv, q).unwrap();
            db_q.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        embed_q.sort_by(|a, b| a.partial_cmp(b).unwrap());
        db_q.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let db_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        println!("--- {n_cards} cards / {n_bullets} bullets ({n_embeds} embeds) ---");
        println!(
            "  IMPORT   embed {:.0} ms ({:.1} ms/embed) + write {:.0} ms = {:.1} s total",
            embed_ms,
            embed_ms / n_embeds as f64,
            write_ms,
            (embed_ms + write_ms) / 1000.0
        );
        println!(
            "  RETRIEVE query-embed p50 {:.1} / p95 {:.1} ms | DB hybrid p50 {:.2} / p95 {:.2} ms | per-tick p95 {:.1} ms",
            pct(&embed_q, 0.50),
            pct(&embed_q, 0.95),
            pct(&db_q, 0.50),
            pct(&db_q, 0.95),
            pct(&embed_q, 0.95) + pct(&db_q, 0.95),
        );
        println!("  DISK     {:.1} MB on disk\n", db_bytes as f64 / 1_048_576.0);
        let _ = std::fs::remove_file(&path);
    }
    println!("Reminder: card-generation from raw text (local LLM) is a SEPARATE cost — projected from the marathon's ~30 tok/s, not measured here.");
}
