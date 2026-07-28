//! One provider for every OpenAI-compatible endpoint: OpenRouter (default),
//! Groq, OpenAI, or any custom base URL. They share the chat/completions wire
//! format; only two axes vary, encoded as small enums (contract verified
//! 2026-07-22, see 10_RESEARCH_LOG):
//!   - token field:  max_tokens (OpenRouter/OpenAI) vs max_completion_tokens (Groq)
//!   - schema mode:  strict json_schema vs json_object fallback
//!
//! The JSON comes back as a STRING in choices[0].message.content → we parse it.

use super::provider::{AssemblyPrompt, Provider, RawAssembly};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub enum TokenField {
    MaxTokens,
    MaxCompletionTokens,
}

#[derive(Debug, Clone, Copy)]
pub enum SchemaMode {
    StrictJsonSchema,
    JsonObject,
}

pub struct OpenAiCompat {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub token_field: TokenField,
    pub schema_mode: SchemaMode,
    pub extra_headers: Vec<(String, String)>,
    http: reqwest::Client,
    provider_name: &'static str,
}

impl OpenAiCompat {
    /// Preset for a provider id. Groq is the only one needing the
    /// max_completion_tokens field and json_object fallback (strict schema is
    /// gpt-oss-only there).
    pub fn preset(provider: &str, api_key: String, model: Option<String>) -> Self {
        let http = reqwest::Client::new();
        match provider {
            "groq" => Self {
                base_url: "https://api.groq.com/openai/v1".into(),
                api_key,
                model: model.unwrap_or_else(|| "openai/gpt-oss-20b".into()),
                token_field: TokenField::MaxCompletionTokens,
                schema_mode: SchemaMode::StrictJsonSchema,
                extra_headers: vec![],
                http,
                provider_name: "groq",
            },
            "openai" => Self {
                base_url: "https://api.openai.com/v1".into(),
                api_key,
                model: model.unwrap_or_else(|| "gpt-4o-mini".into()),
                token_field: TokenField::MaxTokens,
                schema_mode: SchemaMode::StrictJsonSchema,
                extra_headers: vec![],
                http,
                provider_name: "openai",
            },
            // Default: OpenRouter.
            _ => Self {
                base_url: "https://openrouter.ai/api/v1".into(),
                api_key,
                model: model.unwrap_or_else(|| "openai/gpt-4o-mini".into()),
                token_field: TokenField::MaxTokens,
                schema_mode: SchemaMode::StrictJsonSchema,
                extra_headers: vec![
                    ("HTTP-Referer".into(), "https://github.com/anchor".into()),
                    ("X-Title".into(), "Anchor".into()),
                ],
                http,
                provider_name: "openrouter",
            },
        }
    }

    /// Custom OpenAI-compatible endpoint the user typed in settings.
    pub fn custom(base_url: String, api_key: String, model: String) -> Self {
        Self {
            base_url,
            api_key,
            model,
            token_field: TokenField::MaxTokens,
            schema_mode: SchemaMode::JsonObject,
            extra_headers: vec![],
            http: reqwest::Client::new(),
            provider_name: "custom",
        }
    }

    fn response_format(&self) -> Value {
        match self.schema_mode {
            SchemaMode::StrictJsonSchema => json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "cue_card",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string" },
                            // maxItems matches MAX_BULLETS so the API path is not
                            // capped tighter than the local one (both keep ≤8).
                            "points": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": crate::mode2::MAX_BULLETS }
                        },
                        "required": ["title", "points"],
                        "additionalProperties": false
                    }
                }
            }),
            SchemaMode::JsonObject => json!({ "type": "json_object" }),
        }
    }

    /// The system prompt to actually send. Under a strict JSON schema the keys
    /// are pinned by the schema, so the base prompt is enough. Under plain
    /// `json_object` the model is only promised *valid JSON* — nothing names our
    /// keys — so a custom endpoint would emit e.g. `{"bullets": [...]}` and the
    /// parse into `Points` fails. Spell out the exact shape, as the local path does.
    fn system_for(&self, prompt: &AssemblyPrompt) -> String {
        match self.schema_mode {
            SchemaMode::StrictJsonSchema => prompt.system().to_string(),
            SchemaMode::JsonObject => format!(
                "{}\nReturn ONLY a JSON object of exactly this shape: \
                 {{\"title\": \"<the question, rephrased>\", \"points\": [\"<keyword bullet>\"]}}. \
                 The key MUST be \"points\" (an array of short strings), 1 to {} items.",
                prompt.system(),
                crate::mode2::MAX_BULLETS
            ),
        }
    }
}

impl OpenAiCompat {
    /// Generic JSON completion (json_object mode — works on every preset).
    /// Used by the ingestion card generator; Mode-2 assembly keeps the strict
    /// schema path in `assemble`.
    pub async fn complete_json(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<String, String> {
        let mut body = json!({
            "model": self.model,
            "temperature": 0,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ],
            "response_format": { "type": "json_object" },
        });
        match self.token_field {
            TokenField::MaxTokens => body["max_tokens"] = json!(max_tokens),
            TokenField::MaxCompletionTokens => body["max_completion_tokens"] = json!(max_tokens),
        }

        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url.trim_end_matches('/')))
            .bearer_auth(&self.api_key)
            .json(&body);
        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("{} request failed: {e}", self.provider_name))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{} {status}: {}", self.provider_name, truncate(&text, 300)));
        }
        let parsed: ChatResponse =
            serde_json::from_str(&text).map_err(|e| format!("response parse: {e}"))?;
        parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| "empty choices".into())
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    message: Message,
}
#[derive(Deserialize)]
struct Message {
    content: String,
}
#[derive(Deserialize)]
struct Points {
    title: String,
    points: Vec<String>,
}

impl Provider for OpenAiCompat {
    fn name(&self) -> &'static str {
        self.provider_name
    }

    async fn assemble(&self, prompt: &AssemblyPrompt) -> Result<RawAssembly, String> {
        let mut body = json!({
            "model": self.model,
            "temperature": 0,
            "messages": [
                { "role": "system", "content": self.system_for(prompt) },
                { "role": "user", "content": prompt.user() }
            ],
            "response_format": self.response_format(),
        });
        let tokens = 220;
        match self.token_field {
            TokenField::MaxTokens => body["max_tokens"] = json!(tokens),
            TokenField::MaxCompletionTokens => body["max_completion_tokens"] = json!(tokens),
        }
        // OpenRouter: only route to providers that honour our params (schema).
        if self.provider_name == "openrouter" {
            body["provider"] = json!({ "require_parameters": true });
        }

        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url.trim_end_matches('/')))
            .bearer_auth(&self.api_key)
            .json(&body);
        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| format!("{} request failed: {e}", self.provider_name))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{} {status}: {}", self.provider_name, truncate(&text, 300)));
        }

        let parsed: ChatResponse =
            serde_json::from_str(&text).map_err(|e| format!("response parse: {e}"))?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .ok_or("empty choices")?;
        let pts: Points = serde_json::from_str(content)
            .map_err(|e| format!("content parse: {e} — was: {}", truncate(content, 200)))?;
        Ok(RawAssembly {
            title: pts.title,
            bullets: pts.points,
        })
    }
}

fn truncate(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}
