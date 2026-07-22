//! Embedded local LLM (llama-cpp-2). Runs a downloaded GGUF in-process — no
//! server, no external UI. The engine lazy-loads the active model and keeps
//! it in memory; inference runs on a blocking thread (llama.cpp is sync).
//!
//! Prompt handling mirrors the model marathon that chose these models: the
//!   model's own chat template, `/no_think` to suppress reasoning dumps, an
//!   explicit "return JSON" instruction, then the first {...} block is parsed.

use super::provider::{AssemblyPrompt, Provider, RawAssembly};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
#[allow(deprecated)]
use llama_cpp_2::model::Special;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One shared backend per process (llama.cpp requires a single init).
fn backend() -> &'static LlamaBackend {
    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    BACKEND.get_or_init(|| LlamaBackend::init().expect("llama backend init"))
}

struct Loaded {
    id: String,
    model: LlamaModel,
}

/// Holds the currently-loaded model. Lazy: nothing loads until the first
/// local assembly. Swapping the active model drops the old one (frees RAM).
#[derive(Default)]
pub struct LocalEngine {
    loaded: Mutex<Option<Loaded>>,
}

impl LocalEngine {
    /// Ensures `id` (at `path`) is the loaded model; loads/swaps if needed.
    fn ensure(&self, id: &str, path: &Path) -> Result<(), String> {
        let mut guard = self.loaded.lock().map_err(|e| e.to_string())?;
        if guard.as_ref().map(|l| l.id.as_str()) == Some(id) {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let model = LlamaModel::load_from_file(backend(), path, &LlamaModelParams::default())
            .map_err(|e| format!("failed to load {id}: {e}"))?;
        tracing::info!(id, elapsed_s = started.elapsed().as_secs_f64(), "local model loaded");
        *guard = Some(Loaded {
            id: id.to_string(),
            model,
        });
        Ok(())
    }

    pub fn unload(&self) {
        if let Ok(mut g) = self.loaded.lock() {
            *g = None;
        }
    }

    /// Blocking generation from already-owned prompt text (safe to call inside
    /// spawn_blocking). The model must already be loaded via `ensure`.
    // token_to_str/Special::Tokenize are deprecated in favour of token_to_piece
    // (which needs an encoding_rs decoder); the simple path is fine for our
    // short, single-shot generations.
    #[allow(deprecated, clippy::explicit_counter_loop)]
    fn generate(&self, sys: &str, user: &str) -> Result<RawAssembly, String> {
        let guard = self.loaded.lock().map_err(|e| e.to_string())?;
        let model = &guard.as_ref().ok_or("no local model loaded")?.model;

        let tmpl = model.chat_template(None).ok();
        // /no_think suppresses reasoning models' <think> dumps.
        let sys = format!(
            "{sys}\n/no_think\nReturn ONLY a JSON object: {{\"title\": \"...\", \"points\": [\"...\"]}}"
        );
        let text = build_prompt(model, tmpl.as_ref(), &sys, user)?;

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(2048))
            .with_n_threads(4)
            .with_n_threads_batch(4);
        let mut ctx = model
            .new_context(backend(), ctx_params)
            .map_err(|e| e.to_string())?;

        let tokens = model
            .str_to_token(&text, AddBos::Always)
            .map_err(|e| e.to_string())?;
        let mut batch = LlamaBatch::new(2048, 1);
        let last = tokens.len().saturating_sub(1);
        for (i, tok) in tokens.iter().enumerate() {
            batch
                .add(*tok, i as i32, &[0], i == last)
                .map_err(|e| e.to_string())?;
        }
        let mut sampler =
            LlamaSampler::chain_simple([LlamaSampler::temp(0.2), LlamaSampler::greedy()]);

        ctx.decode(&mut batch).map_err(|e| e.to_string())?;
        let mut out = String::new();
        let mut n_cur = batch.n_tokens();
        for _ in 0..220 {
            let tok = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(tok);
            if model.is_eog_token(tok) {
                break;
            }
            out.push_str(&model.token_to_str(tok, Special::Tokenize).unwrap_or_default());
            batch.clear();
            batch.add(tok, n_cur, &[0], true).map_err(|e| e.to_string())?;
            n_cur += 1;
            ctx.decode(&mut batch).map_err(|e| e.to_string())?;
        }

        parse_assembly(&out)
    }
}

/// A thin Provider wrapper carrying the model id + path to load.
pub struct Local {
    engine: Arc<LocalEngine>,
    id: String,
    path: PathBuf,
}

impl Local {
    pub fn new(engine: Arc<LocalEngine>, id: String, path: PathBuf) -> Self {
        Self { engine, id, path }
    }
}

impl Provider for Local {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn assemble(&self, prompt: &AssemblyPrompt) -> Result<RawAssembly, String> {
        self.engine.ensure(&self.id, &self.path)?;
        // llama.cpp is blocking — run it off the async runtime's worker, with
        // owned prompt text so nothing borrows across the thread boundary.
        let engine = self.engine.clone();
        let sys = prompt.system();
        let user = prompt.user();
        tauri::async_runtime::spawn_blocking(move || engine.generate(&sys, &user))
            .await
            .map_err(|e| format!("local generate task failed: {e}"))?
    }
}

fn build_prompt(
    model: &LlamaModel,
    tmpl: Option<&LlamaChatTemplate>,
    sys: &str,
    user: &str,
) -> Result<String, String> {
    let chat = vec![
        LlamaChatMessage::new("system".into(), sys.into()).map_err(|e| e.to_string())?,
        LlamaChatMessage::new("user".into(), user.into()).map_err(|e| e.to_string())?,
    ];
    match tmpl {
        Some(t) => model
            .apply_chat_template(t, &chat, true)
            .map_err(|e| e.to_string()),
        None => Ok(format!(
            "<|im_start|>system\n{sys}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n"
        )),
    }
}

fn parse_assembly(out: &str) -> Result<RawAssembly, String> {
    let json = extract_json(out).ok_or("model produced no JSON object")?;
    let parsed: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!("local JSON parse: {e}"))?;
    let title = parsed
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Unexpected question")
        .to_string();
    let bullets = parsed
        .get("points")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(RawAssembly { title, bullets })
}

fn extract_json(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
