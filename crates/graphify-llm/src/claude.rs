//! Claude direct API backend (Anthropic Messages API via `ureq`).
//!
//! Ports the `_call_claude` function in `graphify-py/graphify/llm.py`.

use serde::Deserialize;
use serde_json::json;

use crate::openai_compat::resolve_max_tokens;
use crate::{
    EXTRACTION_SYSTEM, LlmBackend, LlmError, LlmResponse, parse_llm_json, response_is_hollow,
};

/// Default model for the Claude backend.
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
/// API key environment variable.
pub const ENV_KEY: &str = "ANTHROPIC_API_KEY";
/// Model override environment variable.
pub const MODEL_ENV_KEY: &str = "GRAPHIFY_CLAUDE_MODEL";

const BASE_URL: &str = "https://api.anthropic.com";

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Option<Vec<AnthropicContent>>,
    usage: Option<AnthropicUsage>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

/// Claude direct API backend.
pub struct ClaudeBackend {
    api_key: String,
}

impl ClaudeBackend {
    /// Create from environment variables (reads `ANTHROPIC_API_KEY`).
    #[must_use]
    pub fn from_env() -> Self {
        let api_key = std::env::var(ENV_KEY).unwrap_or_default();
        Self { api_key }
    }

    /// Create with an explicit API key.
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl LlmBackend for ClaudeBackend {
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "claude"
    }

    /// Dispatches to [`call_claude`] using the stored API key.
    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        call_claude(&self.api_key, model, messages, max_tokens)
    }

    /// Delegates to the shared tiktoken-based estimator.
    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Call Anthropic Messages API and return an [`LlmResponse`].
///
/// # Errors
/// Returns [`LlmError::Security`] if the API URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_claude(
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> Result<LlmResponse, LlmError> {
    graphify_security::validate_url(BASE_URL)?;

    let endpoint = format!("{BASE_URL}/v1/messages");
    let timeout = crate::openai_compat::api_timeout();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();

    let body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": EXTRACTION_SYSTEM,
        "messages": messages,
    });

    let http_resp = agent
        .post(&endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| LlmError::Http(e.to_string()))?;

    let resp: AnthropicResponse = http_resp
        .into_body()
        .read_json()
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    let raw_content = resp
        .content
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.text.clone());

    let content_str = raw_content.as_deref().unwrap_or("{}");
    let mut parsed = parse_llm_json(content_str);

    let input_tokens = resp
        .usage
        .as_ref()
        .and_then(|u| u.input_tokens)
        .unwrap_or(0);
    let output_tokens = resp
        .usage
        .as_ref()
        .and_then(|u| u.output_tokens)
        .unwrap_or(0);

    let finish_reason_raw = resp.stop_reason.as_deref().unwrap_or("end_turn");
    let mut finish_reason = if finish_reason_raw == "max_tokens" {
        "length".to_string()
    } else {
        "stop".to_string()
    };

    if response_is_hollow(raw_content.as_deref(), &parsed) && finish_reason != "length" {
        eprintln!(
            "[graphify] claude returned a hollow response; treating as \
             truncation so adaptive retry can bisect the chunk."
        );
        finish_reason = "length".to_string();
    }

    parsed["input_tokens"] = json!(input_tokens);
    parsed["output_tokens"] = json!(output_tokens);
    parsed["model"] = json!(model);
    parsed["finish_reason"] = json!(&finish_reason);

    Ok(LlmResponse {
        nodes: parsed["nodes"].as_array().cloned().unwrap_or_default(),
        edges: parsed["edges"].as_array().cloned().unwrap_or_default(),
        hyperedges: parsed["hyperedges"].as_array().cloned().unwrap_or_default(),
        input_tokens,
        output_tokens,
        model: model.to_string(),
        finish_reason,
        elapsed_seconds: 0.0,
        failed_chunk_indices: vec![],
    })
}

/// Return resolved max tokens for Claude (honours env override).
#[must_use]
pub fn default_max_tokens() -> u32 {
    resolve_max_tokens(16_384)
}
