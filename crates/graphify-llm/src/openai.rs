//! `OpenAI` backend.
//!
//! Ports the openai section in `graphify-py/graphify/llm.py`.

use crate::kimi::call_plain_openai_compat;
use crate::openai_compat::{OpenAiRequest, api_timeout, call_openai_compat, resolve_max_tokens};
use crate::{LlmBackend, LlmError, LlmResponse};

/// Default model.
pub const DEFAULT_MODEL: &str = "gpt-4.1-mini";
/// API key env var.
pub const ENV_KEY: &str = "OPENAI_API_KEY";
/// Model override env var.
pub const MODEL_ENV_KEY: &str = "GRAPHIFY_OPENAI_MODEL";
/// Base URL override env var (defaults to `https://api.openai.com/v1`).
pub const BASE_URL_ENV_KEY: &str = "GRAPHIFY_OPENAI_BASE_URL";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Effective base URL, honouring [`BASE_URL_ENV_KEY`] when set.
#[must_use]
pub fn base_url() -> String {
    std::env::var(BASE_URL_ENV_KEY)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// `OpenAI` backend.
pub struct OpenAiBackend {
    api_key: String,
}

impl OpenAiBackend {
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

impl LlmBackend for OpenAiBackend {
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "openai"
    }

    /// Dispatches to [`call_openai`] using the stored API key.
    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        call_openai(&self.api_key, model, messages, max_tokens)
    }

    /// Delegates to the shared tiktoken-based estimator.
    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Call `OpenAI` API.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_openai(
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> Result<LlmResponse, LlmError> {
    let base = base_url();
    let req = OpenAiRequest {
        base_url: &base,
        api_key,
        model,
        messages: messages.to_vec(),
        temperature: Some(0.0),
        reasoning_effort: None,
        max_completion_tokens: max_tokens,
        disable_thinking: false,
        ollama_options: None,
        backend_name: "openai",
        timeout: api_timeout(),
    };
    call_openai_compat(&req)
}

/// Plain-text call for the LLM tiebreaker path.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_openai_plain(
    api_key: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
) -> Result<String, LlmError> {
    let base = base_url();
    call_plain_openai_compat(&crate::kimi::PlainOpenAiRequest {
        base_url: &base,
        api_key,
        model,
        prompt,
        temperature: Some(0.0),
        reasoning_effort: None,
        disable_thinking: false,
        max_tokens,
    })
}

/// Default max tokens for openai.
#[must_use]
pub fn default_max_tokens() -> u32 {
    resolve_max_tokens(8_192)
}
