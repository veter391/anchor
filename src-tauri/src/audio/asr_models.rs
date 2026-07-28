//! First-run ASR model downloader. Unlike the Mode-2 LLM (one GGUF file), a
//! sherpa-onnx ASR model is a *set* of files (encoder / decoder / joiner int8
//! ONNX + tokens.txt) that must land together in `data/models/<dir_name>/`,
//! where `asr::model_dir_named` already resolves them. Mirrors the hardened
//! single-file downloader in `mode2::models` (sha256 per file, `.part` temp,
//! stall timeout, in-flight guard) but streams a whole bundle with one
//! aggregate progress figure, and is resumable: a file already present with the
//! right size is skipped, so an interrupted 660 MB download continues instead
//! of restarting.
//!
//! The sha256 + size of every file are the ground truth measured from the exact
//! bundles the app was validated with (Phase 7); the download is rejected on any
//! mismatch, so a user always gets byte-identical model files or a clear error.

use crate::audio::{asr, asr_offline};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One file inside a model bundle.
pub struct AsrFile {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
}

/// A downloadable ASR model. `id` matches the engine-preference name
/// (`multilingual` / `english` / `offline`) so the picker and the engine agree;
/// `dir_name` is the on-disk folder `asr::model_dir_named` looks for.
pub struct AsrModelInfo {
    pub id: &'static str,
    pub dir_name: &'static str,
    pub name: &'static str,
    pub tagline: &'static str,
    pub languages: &'static str,
    pub licence: &'static str,
    pub files: &'static [AsrFile],
}

impl AsrModelInfo {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size_bytes).sum()
    }
}

// ── Verified download sources (HF loose files) ──────────────────────
// Every URL + sha256 + size below was independently verified: the HF LFS oid
// of each .onnx and the raw sha256 of each tokens.txt were fetched and matched
// byte-for-byte against the exact bundles the app was validated with (Phase 7).
// Note the multilingual repo owner is `csukuangfj2` (with a 2); the other two
// are plain `csukuangfj`. sherpa-onnx publishes no per-asset archive checksum,
// so these loose files (each sha256-pinned) are the integrity anchor.
const ML_ENC_URL: &str = "https://huggingface.co/csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-320ms-int8-2026-06-11/resolve/main/encoder.int8.onnx";
const ML_DEC_URL: &str = "https://huggingface.co/csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-320ms-int8-2026-06-11/resolve/main/decoder.int8.onnx";
const ML_JOI_URL: &str = "https://huggingface.co/csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-320ms-int8-2026-06-11/resolve/main/joiner.int8.onnx";
const ML_TOK_URL: &str = "https://huggingface.co/csukuangfj2/sherpa-onnx-nemotron-3.5-asr-streaming-0.6b-320ms-int8-2026-06-11/resolve/main/tokens.txt";
const EN_ENC_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemotron-speech-streaming-en-0.6b-int8-2026-01-14/resolve/main/encoder.int8.onnx";
const EN_DEC_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemotron-speech-streaming-en-0.6b-int8-2026-01-14/resolve/main/decoder.int8.onnx";
const EN_JOI_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemotron-speech-streaming-en-0.6b-int8-2026-01-14/resolve/main/joiner.int8.onnx";
const EN_TOK_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemotron-speech-streaming-en-0.6b-int8-2026-01-14/resolve/main/tokens.txt";
const PK_ENC_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/encoder.int8.onnx";
const PK_DEC_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/decoder.int8.onnx";
const PK_JOI_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/joiner.int8.onnx";
const PK_TOK_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/tokens.txt";

/// The three selectable ASR models. Order = display order (default first).
/// sha256 + size are ground truth measured from the validated bundles.
pub const REGISTRY: &[AsrModelInfo] = &[
    AsrModelInfo {
        id: "multilingual",
        dir_name: asr::ASR_MODEL_MULTILINGUAL,
        name: "Multilingual",
        tagline: "EN · ES · RU · UK · DE",
        languages: "English, Spanish, Russian, Ukrainian, German",
        licence: "OpenMDW-1.1",
        files: &[
            AsrFile {
                name: "encoder.int8.onnx",
                url: ML_ENC_URL,
                sha256: "f79c3fcc149f268b54b7d5754bdc2ba5c47c16b1fc70d15728a56f6efbf60ca5",
                size_bytes: 657_601_518,
            },
            AsrFile {
                name: "decoder.int8.onnx",
                url: ML_DEC_URL,
                sha256: "19f9c98fc6d0a2c33a65a43b36fdb2e914c26c0aa9764be3aebc502a1e982fb0",
                size_bytes: 14_978_075,
            },
            AsrFile {
                name: "joiner.int8.onnx",
                url: ML_JOI_URL,
                sha256: "4101c7c679a0bc30483794b27a059e34e79232aa2068d78d51231a22c8b0d7ce",
                size_bytes: 9_504_438,
            },
            AsrFile {
                name: "tokens.txt",
                url: ML_TOK_URL,
                sha256: "729cc103155bafa785f9cd45746cd41cabe97eab7182fc04d594129587958f8a",
                size_bytes: 131_440,
            },
        ],
    },
    AsrModelInfo {
        id: "english",
        dir_name: asr::ASR_MODEL_EN,
        name: "English — fastest",
        tagline: "EN only, lightest load",
        languages: "English",
        licence: "OpenMDW-1.1",
        files: &[
            AsrFile {
                name: "encoder.int8.onnx",
                url: EN_ENC_URL,
                sha256: "2f6ae81fe4ccd69ef04cdf048ecd49628e2d3148a6195e152a91b4d2497952dc",
                size_bytes: 652_916_830,
            },
            AsrFile {
                name: "decoder.int8.onnx",
                url: EN_DEC_URL,
                sha256: "1fb1795cb46e7d0e99b2e096eae83f7e324294e895975a1a894b0384cbbe37f6",
                size_bytes: 7_257_753,
            },
            AsrFile {
                name: "joiner.int8.onnx",
                url: EN_JOI_URL,
                sha256: "a3f41dccc0f67f37e4210051d1c39a29d473c841cfc32fe574135bac890db91d",
                size_bytes: 1_735_862,
            },
            AsrFile {
                name: "tokens.txt",
                url: EN_TOK_URL,
                sha256: "dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff",
                size_bytes: 8_952,
            },
        ],
    },
    AsrModelInfo {
        id: "offline",
        dir_name: asr_offline::ASR_MODEL_PARAKEET,
        name: "Compatibility (offline)",
        tagline: "25 European languages, weak CPUs",
        languages: "25 European languages",
        licence: "CC-BY-4.0",
        files: &[
            AsrFile {
                name: "encoder.int8.onnx",
                url: PK_ENC_URL,
                sha256: "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
                size_bytes: 652_184_281,
            },
            AsrFile {
                name: "decoder.int8.onnx",
                url: PK_DEC_URL,
                sha256: "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
                size_bytes: 11_845_275,
            },
            AsrFile {
                name: "joiner.int8.onnx",
                url: PK_JOI_URL,
                sha256: "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
                size_bytes: 6_355_277,
            },
            AsrFile {
                name: "tokens.txt",
                url: PK_TOK_URL,
                sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                size_bytes: 93_939,
            },
        ],
    },
];

pub fn find(id: &str) -> Option<&'static AsrModelInfo> {
    REGISTRY.iter().find(|m| m.id == id)
}

fn model_dir(app_data: &Path, info: &AsrModelInfo) -> PathBuf {
    crate::mode2::models::models_dir(app_data).join(info.dir_name)
}

/// A single file is present and the right size (cheap; sha256 is verified at
/// download time, not on every startup).
fn file_ok(path: &Path, size: u64) -> bool {
    std::fs::metadata(path).map(|m| m.len() == size).unwrap_or(false)
}

/// Installed = every file present with the expected size.
pub fn is_installed(app_data: &Path, info: &AsrModelInfo) -> bool {
    let dir = model_dir(app_data, info);
    info.files.iter().all(|f| file_ok(&dir.join(f.name), f.size_bytes))
}

/// Ids with a download in flight — a second concurrent download of the same
/// bundle would interleave writes into the same `.part` files.
fn inflight() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static INFLIGHT: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    INFLIGHT.get_or_init(Default::default)
}

/// Download every file of a bundle into `data/models/<dir_name>/`, verifying
/// each against its sha256. `on_progress(done, total)` reports bytes across the
/// whole bundle. Already-present files (right size) are skipped, so an
/// interrupted download resumes rather than restarting.
pub async fn download(
    app_data: &Path,
    info: &AsrModelInfo,
    on_progress: impl Fn(u64, u64),
) -> Result<(), String> {
    {
        let mut set = inflight().lock().map_err(|e| e.to_string())?;
        if !set.insert(info.id.to_string()) {
            return Err(format!("{} is already downloading", info.id));
        }
    }
    let result = download_inner(app_data, info, on_progress).await;
    if let Ok(mut set) = inflight().lock() {
        set.remove(info.id);
    }
    result
}

async fn download_inner(
    app_data: &Path,
    info: &AsrModelInfo,
    on_progress: impl Fn(u64, u64),
) -> Result<(), String> {
    let dir = model_dir(app_data, info);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let total = info.total_bytes();

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        // Model URLs are hardcoded https://huggingface.co/... — refuse any
        // downgrade to http:// (e.g. via a redirect) so bytes only ever arrive
        // over TLS before the sha256 gate.
        .https_only(true)
        .build()
        .map_err(|e| e.to_string())?;

    // Bytes fully committed in previously-finished files — the base the current
    // file's own progress is added to for one continuous 0→total figure.
    let mut base: u64 = 0;
    for file in info.files {
        let final_path = dir.join(file.name);
        if file_ok(&final_path, file.size_bytes) {
            base += file.size_bytes;
            on_progress(base, total);
            continue;
        }
        download_file(&client, file, &dir, base, total, &on_progress).await?;
        base += file.size_bytes;
        on_progress(base, total);
    }
    Ok(())
}

async fn download_file(
    client: &reqwest::Client,
    file: &AsrFile,
    dir: &Path,
    base: u64,
    total: u64,
    on_progress: &impl Fn(u64, u64),
) -> Result<(), String> {
    use futures_util::StreamExt;

    if file.url.is_empty() {
        return Err(format!("no download URL configured for {}", file.name));
    }
    use std::io::Read;
    let tmp_path = dir.join(format!("{}.part", file.name));
    // Corruption (oversize / hash mismatch) deletes the .part so the next try is
    // clean; a NETWORK failure keeps it so the next try resumes (see below).
    let drop_part = || {
        let _ = std::fs::remove_file(&tmp_path);
    };

    // Resume an interrupted download: if a .part from a killed run is present and
    // shorter than the target, ask for the rest with a Range request.
    let have = std::fs::metadata(&tmp_path).map(|m| m.len()).unwrap_or(0);
    let want_resume = have > 0 && have < file.size_bytes;
    let mut req = client.get(file.url);
    if want_resume {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("download request failed for {}: {e}", file.name))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("download HTTP {status} for {}", file.name));
    }
    // 206 = the server honoured the range → append; anything else (200) is a
    // full body → start the file over.
    let resuming = want_resume && status == reqwest::StatusCode::PARTIAL_CONTENT;

    let mut hasher = Sha256::new();
    let mut cur: u64;
    let mut out = if resuming {
        // Seed the hash with the bytes already on disk, then append to them.
        let mut existing = std::fs::File::open(&tmp_path).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = existing.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        cur = have;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&tmp_path)
            .map_err(|e| e.to_string())?
    } else {
        cur = 0;
        std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?
    };
    on_progress(base + cur, total);

    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::time::timeout(std::time::Duration::from_secs(60), stream.next()).await;
        let Ok(item) = next else {
            // Network stall — keep the .part so the next attempt resumes.
            return Err(format!("download stalled for {} (no data for 60 s)", file.name));
        };
        let Some(chunk) = item else { break };
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => return Err(format!("download stream error for {}: {e}", file.name)),
        };
        cur += chunk.len() as u64;
        // Never write past the file's known size. The sha256 below is the real
        // integrity gate, but a compromised/MITM'd transport could stream
        // unbounded data straight to disk before that check runs — cap it at the
        // ground-truth size (a genuine file matches exactly, so this never trips
        // on a good download).
        if cur > file.size_bytes {
            drop_part();
            return Err(format!(
                "download for {} exceeded its known size ({} B) — rejected",
                file.name, file.size_bytes
            ));
        }
        hasher.update(&chunk);
        if let Err(e) = out.write_all(&chunk) {
            return Err(e.to_string());
        }
        on_progress(base + cur, total);
    }
    if let Err(e) = out.flush() {
        return Err(e.to_string());
    }
    drop(out);

    let digest = format!("{:x}", hasher.finalize());
    if digest != file.sha256 {
        drop_part(); // corrupt → force a clean re-download next time
        return Err(format!(
            "sha256 mismatch for {} (got {}…, expected {}…) — download rejected",
            file.name,
            &digest[..12],
            &file.sha256[..12]
        ));
    }
    std::fs::rename(&tmp_path, dir.join(file.name)).map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove a downloaded bundle (the four files; leaves the dir if a user dropped
/// extras like README/test_wavs in it, which is fine).
pub fn delete(app_data: &Path, info: &AsrModelInfo) -> Result<(), String> {
    let dir = model_dir(app_data, info);
    for f in info.files {
        let p = dir.join(f.name);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
