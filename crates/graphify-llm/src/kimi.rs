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
/// Base URL override env var (test redirect).
pub const BASE_URL_ENV_KEY: &str = "GRAPHIFY_KIMI_BASE_URL";
/// Upstream Moonshot/Kimi base-URL env var (#1458): points the backend at any
/// OpenAI-compatible server (`LiteLLM`, self-hosted proxy, …).
pub const KIMI_BASE_URL_ENV: &str = "KIMI_BASE_URL";
const DEFAULT_BASE_URL: &str = "https://api.moonshot.ai/v1";

/// Effective base URL: [`BASE_URL_ENV_KEY`] (test redirect) then
/// [`KIMI_BASE_URL_ENV`], else Moonshot's official endpoint.
#[must_use]
pub fn base_url() -> String {
    std::env::var(BASE_URL_ENV_KEY)
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var(KIMI_BASE_URL_ENV)
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

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
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "kimi"
    }

    /// Dispatches to [`call_kimi`] using the stored API key.
    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        call_kimi(&self.api_key, model, messages, max_tokens)
    }

    /// Delegates to the shared tiktoken-based estimator.
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
        base_url: &base_url(),
        api_key,
        model,
        messages: messages.to_vec(),
        temperature: Some(0.0),
        reasoning_effort: None,
        max_completion_tokens: max_tokens,
        // Kimi-k2.6 is a reasoning model — disable thinking.
        disable_thinking: true,
        custom_extra_body: None,
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

/// Arguments for [`call_plain_openai_compat`].
pub(crate) struct PlainOpenAiRequest<'a> {
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    pub prompt: &'a str,
    pub temperature: Option<f64>,
    pub reasoning_effort: Option<&'a str>,
    pub disable_thinking: bool,
    /// Custom-provider `extra_body` passthrough (#7477b46); when `Some` it owns
    /// the request's `extra_body`, overriding [`Self::disable_thinking`].
    pub extra_body: Option<&'a serde_json::Value>,
    pub max_tokens: u32,
}

/// Low-level plain-text `OpenAI`-compat call (returns raw content string).
///
/// Used by other backends (Gemini, `OpenAI`, `DeepSeek`, Ollama) that share the
/// same HTTP shape but different base URLs.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub(crate) fn call_plain_openai_compat(req: &PlainOpenAiRequest<'_>) -> Result<String, LlmError> {
    graphify_security::validate_url(req.base_url)?;

    let messages = plain_messages(req.prompt);
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "max_completion_tokens": req.max_tokens,
        // Force a single non-streamed response (#1223): this path feeds the
        // `--dedup-llm` tiebreaker and is always read as `resp.choices[0]`.
        "stream": false,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(re) = req.reasoning_effort {
        body["reasoning_effort"] = json!(re);
    }
    // A custom provider's explicit extra_body owns the request shape and wins
    // over the moonshot `thinking: disabled` default (#7477b46).
    if let Some(custom) = req.extra_body {
        body["extra_body"] = custom.clone();
    } else if req.disable_thinking {
        body["extra_body"] = json!({"thinking": {"type": "disabled"}});
    }

    let endpoint = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(api_timeout()))
        .build()
        .into();

    let resp: PlainResp = crate::openai_compat::send_json_with_retry(|| {
        agent
            .post(&endpoint)
            .header("Authorization", &format!("Bearer {}", req.api_key))
            .header("Content-Type", "application/json")
            .send_json(&body)
    })
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
