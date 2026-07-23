//! Local embeddings. One multilingual model, one vector space for both the
//! rolling window (queries) and cards/bullets (passages) — see
//! Documents/04_MODELS.md. EmbeddingGemma uses task-prompt prefixes and
//! Matryoshka truncation: we embed at 768d and store the first `dims`
//! components, re-normalized.

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::sync::Mutex;

pub const MODEL_ID: &str = "embeddinggemma-300m-q";
pub const DIMS: usize = 256;

/// EmbeddingGemma prompt formats (Google model card). Verified against the
/// fastembed source: it does NOT auto-prefix Gemma inputs, so these are the
/// only prompts applied — no double-prefixing.
fn query_prompt(text: &str) -> String {
    format!("task: search result | query: {text}")
}
fn passage_prompt(title: &str, text: &str) -> String {
    let title = if title.is_empty() { "none" } else { title };
    format!("title: {title} | text: {text}")
}

pub struct Embedder {
    model: Mutex<Option<TextEmbedding>>,
}

impl Default for Embedder {
    fn default() -> Self {
        Self::new()
    }
}

impl Embedder {
    pub fn new() -> Self {
        Self {
            model: Mutex::new(None),
        }
    }

    /// Loads the model on first use (first run downloads it to the local
    /// fastembed cache). Called from a command thread, never the UI thread.
    fn with_model<T>(
        &self,
        f: impl FnOnce(&mut TextEmbedding) -> anyhow::Result<T>,
    ) -> Result<T, String> {
        let mut guard = self.model.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let started = std::time::Instant::now();
            let model = TextEmbedding::try_new(TextInitOptions::new(
                EmbeddingModel::EmbeddingGemma300MQ,
            ))
            .map_err(|e| format!("embedding model init failed: {e}"))?;
            tracing::info!(
                elapsed_s = started.elapsed().as_secs_f64(),
                "embedding model ready"
            );
            *guard = Some(model);
        }
        f(guard.as_mut().unwrap()).map_err(|e| e.to_string())
    }

    pub fn is_loaded(&self) -> bool {
        self.model.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>, String> {
        let prompts = vec![query_prompt(text)];
        self.with_model(|m| m.embed(prompts, None))
            .map(|mut v| truncate_normalize(v.remove(0)))
    }

    /// (title, text) pairs — cards embed as title+bullets, bullets as bare text.
    pub fn embed_passages(&self, items: &[(String, String)]) -> Result<Vec<Vec<f32>>, String> {
        let prompts: Vec<String> = items
            .iter()
            .map(|(title, text)| passage_prompt(title, text))
            .collect();
        self.with_model(|m| m.embed(prompts, None))
            .map(|vs| vs.into_iter().map(truncate_normalize).collect())
    }
}

/// Matryoshka: keep the first DIMS components, re-normalize to unit length.
fn truncate_normalize(mut v: Vec<f32>) -> Vec<f32> {
    v.truncate(DIMS);
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// sqlite-vec expects raw little-endian f32 blobs.
pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Inverse of `vec_to_blob` — reads a stored embedding back out of vec0.
pub fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
