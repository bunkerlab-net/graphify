//! Kimi K2 backend (`OpenAI`-compatible endpoint via Moonshot AI).
//!
//! Ports the kimi section in `graphify-py/graphify/llm.py`.

use serde::Deserialize;
use serde_json::json;

use crate::openai_compat::{
    OpenAiRequest, api_timeout, call_openai_compat, plain_messages, resolve_max_tokens,
};
use crate::{LlmBackend, LlmError, LlmResponse};

/// Default model.
pub const DEFAULT_MODEL: &str = "kimi-k2.6";
/// API key env var.
pub const ENV_KEY: &str = "MOONSHOT_API_KEY";
const BASE_URL: &str = "https://api.moonshot.ai/v1";

// Response types for plain calls — defined at module scope (not inside fn).
#[derive(Deserialize)]
struct PlainResp {
    choices: Vec<PlainChoice>,
}

#[derive(Deserialize)]
struct PlainChoice {
    message: Option<PlainMsg>,
}

#[derive(Deserialize)]
struct PlainMsg {
    content: Option<String>,
}

/// Kimi K2 backend.
pub struct KimiBackend {
    api_key: String,
}

impl KimiBackend {
    /// Create from environment.
    #[must_use]
    pub fn from_env() -> Self {
        let api_key = std::env::var(ENV_KEY).unwrap_or_default();
        Self { api_key }
    }

    /// Create with explicit API key.
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl LlmBackend for KimiBackend {
    fn name(&self) -> &'static str {
        "kimi"
    }

    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        call_kimi(&self.api_key, model, messages, max_tokens)
    }

    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Call Kimi via `OpenAI`-compat layer.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_kimi(
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> Result<LlmResponse, LlmError> {
    let req = OpenAiRequest {
        base_url: BASE_URL,
        api_key,
        model,
        messages: messages.to_vec(),
        temperature: Some(0.0),
        reasoning_effort: None,
        max_completion_tokens: max_tokens,
        // Kimi-k2.6 is a reasoning model — disable thinking.
        disable_thinking: true,
        ollama_options: None,
        backend_name: "kimi",
        timeout: api_timeout(),
    };
    call_openai_compat(&req)
}

/// Default max tokens for kimi.
#[must_use]
pub fn default_max_tokens() -> u32 {
    resolve_max_tokens(16_384)
}

/// Low-level plain-text `OpenAI`-compat call (returns raw content string).
///
/// Used by other backends (Gemini, `OpenAI`, `DeepSeek`, Ollama) that share the
/// same HTTP shape but different base URLs.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
#[allow(clippy::too_many_arguments)]
pub(crate) fn call_plain_openai_compat(
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    temperature: Option<f64>,
    reasoning_effort: Option<&str>,
    disable_thinking: bool,
    max_tokens: u32,
) -> Result<String, LlmError> {
    graphify_security::validate_url(base_url)?;

    let messages = plain_messages(prompt);
    let mut body = json!({
        "model": model,
        "messages": messages,
        "max_completion_tokens": max_tokens,
    });
    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(re) = reasoning_effort {
        body["reasoning_effort"] = json!(re);
    }
    if disable_thinking {
        body["extra_body"] = json!({"thinking": {"type": "disabled"}});
    }

    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(api_timeout()))
        .build()
        .into();

    let resp: PlainResp = agent
        .post(&endpoint)
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| LlmError::Http(e.to_string()))?
        .into_body()
        .read_json()
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    Ok(resp
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message)
        .and_then(|m| m.content)
        .unwrap_or_default())
}
