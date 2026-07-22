//! One-off calibration probe for the Mode-2 question-relevance gate: prints
//! cosine(question, bullet) for known-relevant vs known-irrelevant pairs, in
//! two schemes (query-query vs query-passage), EN and cross-lingual RU.
//! The REL_FLOOR constant in mode2/mod.rs is chosen from these numbers.
//!
//! Run:  build-env.bat cargo run --manifest-path src-tauri\Cargo.toml
//!         --example rel_probe --no-default-features -j 2

use anchor::embed::Embedder;

const CASES: &[(&str, &[(&str, bool)])] = &[
    (
        "what do you like to do outside of work for fun",
        &[
            ("hobbies: hiking, photography, chess", true),
            ("walks, puzzles, small side projects", true),
            ("reading, gym three times a week", true),
            ("love building things", true), // borderline honest bridge
            ("Helm + ArgoCD, full GitOps", false),
            ("moved 40 services, zero downtime", false),
            ("total comp, not base only", false),
            ("researched the market range", false),
        ],
    ),
    (
        "how did the kubernetes migration go",
        &[
            ("moved 40 services, zero downtime", true),
            ("Helm + ArgoCD, full GitOps", true),
            ("cut infra cost 35 percent", true),
            ("hobbies: hiking, photography, chess", false),
            ("total comp, not base only", false),
            ("good question, love building", false),
        ],
    ),
    (
        // Cross-lingual: RU question, EN bullets (first-class product case).
        "чем вы любите заниматься в свободное время вне работы",
        &[
            ("hobbies: hiking, photography, chess", true),
            ("walks, puzzles, small side projects", true),
            ("Helm + ArgoCD, full GitOps", false),
            ("moved 40 services, zero downtime", false),
        ],
    ),
    (
        "what are your salary expectations",
        &[
            ("researched the market range", true),
            ("total comp, not base only", true),
            ("flexible for the right role", true),
            ("blue-green rollout, no user impact", false),
            ("hobbies: hiking, photography, chess", false),
        ],
    ),
];

fn cos(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x * y) as f64).sum()
}

fn main() {
    let e = Embedder::new();
    let _ = e.embed_query("warmup").unwrap();

    let mut rel_qq: Vec<f64> = vec![];
    let mut irr_qq: Vec<f64> = vec![];
    let mut rel_qp: Vec<f64> = vec![];
    let mut irr_qp: Vec<f64> = vec![];

    for (q, bullets) in CASES {
        let qv = e.embed_query(q).unwrap();
        println!("\nQ: {q}");
        for (b, relevant) in *bullets {
            let bq = e.embed_query(b).unwrap();
            let bp = &e
                .embed_passages(&[(String::new(), b.to_string())])
                .unwrap()[0];
            let qq = cos(&qv, &bq);
            let qp = cos(&qv, bp);
            println!(
                "  {} qq {:.3}  qp {:.3}  | {b}",
                if *relevant { "REL" } else { "IRR" },
                qq,
                qp
            );
            if *relevant {
                rel_qq.push(qq);
                rel_qp.push(qp);
            } else {
                irr_qq.push(qq);
                irr_qp.push(qp);
            }
        }
    }

    let min = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
    let max = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max);
    println!("\n== separation ==");
    println!(
        "query-query : REL min {:.3} | IRR max {:.3} | gap {:+.3}",
        min(&rel_qq),
        max(&irr_qq),
        min(&rel_qq) - max(&irr_qq)
    );
    println!(
        "query-passage: REL min {:.3} | IRR max {:.3} | gap {:+.3}",
        min(&rel_qp),
        max(&irr_qp),
        min(&rel_qp) - max(&irr_qp)
    );
}
