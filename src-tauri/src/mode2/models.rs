//! Local-model registry + downloader. The three winners of the model
//! marathon (2026-07-22), user-selectable. Downloaded into the app's own
//! data dir (never bundled — licences + size), with a sha256 integrity
//! check and a progress event. No external tool, no separate UI.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// Two-or-three-word strength, shown in the picker.
    pub tagline: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub url: &'static str,
    pub is_default: bool,
    pub licence: &'static str,
}

/// The three marathon winners. Order = display order (default first).
pub const REGISTRY: &[ModelInfo] = &[
    ModelInfo {
        id: "qwen3-1.7b",
        name: "Qwen3 1.7B",
        tagline: "Fast & balanced",
        size_bytes: 1_107_409_472,
        sha256: "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
        url: "https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
        is_default: true,
        licence: "Apache-2.0",
    },
    ModelInfo {
        id: "phi4-mini",
        name: "Phi-4 mini",
        tagline: "Most honest",
        size_bytes: 2_491_874_272,
        sha256: "88c00229914083cd112853aab84ed51b87bdf6b9ce42f532d8c85c7c63b1730a",
        url: "https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-Q4_K_M.gguf",
        is_default: false,
        licence: "MIT",
    },
    ModelInfo {
        id: "nuextract2-2b",
        name: "NuExtract 2.0",
        tagline: "Strictly your notes",
        size_bytes: 986_051_360,
        sha256: "4725ac4d8437b8657005de6c5fb2de8dea186e654e9331a092f1c1146bc9582e",
        url: "https://huggingface.co/numind/NuExtract-2.0-2B-GGUF/resolve/main/NuExtract-2.0-2B-Q4_K_M.gguf",
        is_default: false,
        licence: "MIT",
    },
];

pub fn find(id: &str) -> Option<&'static ModelInfo> {
    REGISTRY.iter().find(|m| m.id == id)
}

pub fn models_dir(app_data: &Path) -> PathBuf {
    app_data.join("models")
}

pub fn model_path(app_data: &Path, id: &str) -> PathBuf {
    models_dir(app_data).join(format!("{id}.gguf"))
}

/// Installed = file exists and matches the expected size (cheap check; the
/// sha256 is verified at download time).
pub fn is_installed(app_data: &Path, info: &ModelInfo) -> bool {
    let p = model_path(app_data, info.id);
    std::fs::metadata(&p)
        .map(|m| m.len() == info.size_bytes)
        .unwrap_or(false)
}

/// Streams the GGUF into app-data with sha256 verification. `on_progress`
/// is called with (downloaded, total) so the UI can show a bar.
pub async fn download(
    app_data: &Path,
    info: &ModelInfo,
    on_progress: impl Fn(u64, u64),
) -> Result<(), String> {
    use futures_util::StreamExt;

    let dir = models_dir(app_data);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let final_path = model_path(app_data, info.id);
    let tmp_path = dir.join(format!("{}.part", info.id));

    let resp = reqwest::Client::new()
        .get(info.url)
        .send()
        .await
        .map_err(|e| format!("download request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(info.size_bytes);

    let mut file = std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("download stream error: {e}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    let digest = format!("{:x}", hasher.finalize());
    if digest != info.sha256 {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!(
            "sha256 mismatch for {} (got {}…, expected {}…) — download rejected",
            info.id,
            &digest[..12],
            &info.sha256[..12]
        ));
    }
    std::fs::rename(&tmp_path, &final_path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete(app_data: &Path, id: &str) -> Result<(), String> {
    let p = model_path(app_data, id);
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}
