//! Eyes-on proof that the first-run ASR downloader works end to end against the
//! real Hugging Face source: download a full bundle into a CLEAN temp folder via
//! the actual `asr_models::download` (multi-file, aggregate progress, per-file
//! sha256), confirm `is_installed`, and load the downloaded files with the real
//! recognizer. If this passes, a stranger's first run works.
//!
//! Run:  build-env.bat cargo run --example asr_download_check [--release] -- [multilingual|english|offline]

use anchor::audio::{asr, asr_models, asr_offline};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "multilingual".to_string());
    let info = asr_models::find(&id).unwrap_or_else(|| panic!("unknown model id: {id}"));

    let app_data = PathBuf::from("target").join("dltest");
    let _ = std::fs::remove_dir_all(&app_data); // start clean, like a fresh install
    println!(
        "downloading {} — {} — {} MB across {} files\n  into {}",
        info.name,
        info.languages,
        info.total_bytes() / 1_048_576,
        info.files.len(),
        app_data.display()
    );

    // `on_progress` is a `Fn`, so dedup the print with interior mutability.
    let last_pct = std::cell::Cell::new(u64::MAX);
    asr_models::download(&app_data, info, |done, total| {
        let pct = (done * 100).checked_div(total).unwrap_or(0);
        if pct != last_pct.get() && pct % 5 == 0 {
            last_pct.set(pct);
            println!("  {pct:3}%   {} / {} MB", done / 1_048_576, total / 1_048_576);
        }
    })
    .await
    .expect("download failed");

    println!("download + per-file sha256 verification OK");
    assert!(
        asr_models::is_installed(&app_data, info),
        "is_installed returned false after a successful download"
    );

    // Prove the downloaded files actually load in the real recognizer.
    let dir = asr::model_dir_named(&app_data, info.dir_name)
        .expect("model_dir_named could not resolve the freshly-downloaded model");
    if id == "offline" {
        asr_offline::ParakeetAsr::load(&dir, 4).expect("ParakeetAsr::load on downloaded files");
    } else {
        asr::Asr::load(&dir, 4).expect("Asr::load on downloaded files");
    }
    println!("recognizer loaded the downloaded bundle OK — first-run download works end to end");
}
