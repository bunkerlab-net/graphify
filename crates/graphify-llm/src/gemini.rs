//! Gemini backend — `OpenAI`-compatible endpoint from Google.
//!
//! Ports the gemini section in `graphify-py/graphify/llm.py`.
//! Accepts either `GEMINI_API_KEY` or `GOOGLE_API_KEY`.

use crate::kimi::call_plain_openai_compat;
use crate::openai_compat::{OpenAiRequest, api_timeout, call_openai_compat, resolve_max_tokens};
use crate::{LlmBackend, LlmError, LlmResponse};

/// Default model.
pub const DEFAULT_MODEL: &str = "gemini-3-flash-preview";
/// Primary API key env var.
pub const ENV_KEY: &str = "GEMINI_API_KEY";
/// Fallback API key env var.
pub const ENV_KEY_FALLBACK: &str = "GOOGLE_API_KEY";
/// Model override env var.
pub const MODEL_ENV_KEY: &str = "GRAPHIFY_GEMINI_MODEL";
const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai/";

/// Gemini backend.
pub struct GeminiBackend {
    api_key: String,
}

impl GeminiBackend {
    /// Create from environment.
    #[must_use]
    pub fn from_env() -> Self {
        let api_key = get_api_key();
        Self { api_key }
    }

    /// Create with explicit API key.
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl LlmBackend for GeminiBackend {
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "gemini"
    }

    /// Dispatches to [`call_gemini`] using the stored API key.
    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        call_gemini(&self.api_key, model, messages, max_tokens)
    }

    /// Delegates to the shared tiktoken-based estimator.
    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Return the first available Gemini API key (`GEMINI_API_KEY`, then `GOOGLE_API_KEY`).
#[must_use]
pub fn get_api_key() -> String {
    std::env::var(ENV_KEY)
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var(ENV_KEY_FALLBACK)
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_default()
}

/// Call Gemini via the `OpenAI`-compat endpoint.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_gemini(
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
        reasoning_effort: Some("low"),
        max_completion_tokens: max_tokens,
        disable_thinking: false,
        ollama_options: None,
        backend_name: "gemini",
        timeout: api_timeout(),
    };
    call_openai_compat(&req)
}

/// Plain-text call for the LLM tiebreaker path.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_gemini_plain(
    api_key: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
) -> Result<String, LlmError> {
    call_plain_openai_compat(
        BASE_URL,
        api_key,
        model,
        prompt,
        Some(0.0),
        Some("low"),
        false,
        max_tokens,
    )
}

/// Default max tokens for gemini.
#[must_use]
pub fn default_max_tokens() -> u32 {
    resolve_max_tokens(16_384)
}
