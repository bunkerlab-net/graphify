//! Azure `OpenAI` Service backend.
//!
//! Ports `_azure_client` / `_call_azure` from `graphify-py/graphify/llm.py`.
//!
//! Azure `OpenAI` is request/response-compatible with the `OpenAI` Chat Completions
//! API, but the transport differs: the URL is deployment-scoped
//! (`{endpoint}/openai/deployments/{model}/chat/completions?api-version=…`) and
//! auth uses an `api-key` header instead of `Authorization: Bearer`. It
//! therefore has its own call path rather than reusing the `openai_compat`
//! client (mirroring the Python `_call_azure` split).

use serde_json::{Value, json};

use crate::openai_compat::{api_timeout, resolve_temperature};
use crate::{LlmError, LlmResponse, parse_llm_json, response_is_hollow};

/// Pricing-table default model (gpt-4o). The effective deployment is resolved
/// from the environment by [`resolve_model`].
pub const DEFAULT_MODEL: &str = "gpt-4o";
/// API key env var.
pub const ENV_KEY: &str = "AZURE_OPENAI_API_KEY";
/// Required resource endpoint env var (e.g. `https://my-resource.openai.azure.com/`).
pub const ENDPOINT_ENV: &str = "AZURE_OPENAI_ENDPOINT";
/// Optional API-version env var.
pub const API_VERSION_ENV: &str = "AZURE_OPENAI_API_VERSION";
/// Optional deployment-name env var (highest-priority model source).
pub const DEPLOYMENT_ENV: &str = "AZURE_OPENAI_DEPLOYMENT";
/// Optional model-override env var.
pub const MODEL_ENV_KEY: &str = "GRAPHIFY_AZURE_MODEL";
/// API version used when [`API_VERSION_ENV`] is unset.
const DEFAULT_API_VERSION: &str = "2024-12-01-preview";

/// Resolve the deployment/model name: `AZURE_OPENAI_DEPLOYMENT` →
/// `GRAPHIFY_AZURE_MODEL` → `gpt-4o`. Mirrors the env-derived `default_model`
/// in Python's `BACKENDS["azure"]`.
#[must_use]
pub fn resolve_model() -> String {
    for key in [DEPLOYMENT_ENV, MODEL_ENV_KEY] {
        if let Ok(v) = std::env::var(key) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    DEFAULT_MODEL.to_string()
}

/// Resolve the API version, honouring [`API_VERSION_ENV`] when set.
#[must_use]
pub fn resolve_api_version() -> String {
    std::env::var(API_VERSION_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_API_VERSION.to_string())
}

/// Resolve and validate the resource endpoint.
///
/// # Errors
/// Returns [`LlmError::InvalidInput`] when [`ENDPOINT_ENV`] is unset/blank.
pub fn resolve_endpoint() -> Result<String, LlmError> {
    let endpoint = std::env::var(ENDPOINT_ENV).unwrap_or_default();
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return Err(LlmError::InvalidInput(
            "Azure OpenAI backend requires AZURE_OPENAI_ENDPOINT to be set \
             (e.g. https://my-resource.openai.azure.com/)."
                .to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Build the deployment-scoped chat-completions URL for `endpoint`/`model`.
#[must_use]
pub fn chat_url(endpoint: &str, model: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    let api_version = resolve_api_version();
    format!("{base}/openai/deployments/{model}/chat/completions?api-version={api_version}")
}

/// Azure `OpenAI` backend.
pub struct AzureBackend {
    api_key: String,
    endpoint: String,
}

impl AzureBackend {
    /// Create from environment (`AZURE_OPENAI_API_KEY` / `AZURE_OPENAI_ENDPOINT`).
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var(ENV_KEY).unwrap_or_default(),
            endpoint: std::env::var(ENDPOINT_ENV).unwrap_or_default(),
        }
    }

    /// Create with explicit credentials (for testing).
    #[must_use]
    pub fn new(api_key: String, endpoint: String) -> Self {
        Self { api_key, endpoint }
    }
}

impl crate::LlmBackend for AzureBackend {
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "azure"
    }

    /// Dispatches to [`call_azure`], resolving the endpoint from the stored
    /// value (or erroring when it is unset).
    fn call(
        &self,
        messages: &[Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        let endpoint = if self.endpoint.trim().is_empty() {
            resolve_endpoint()?
        } else {
            self.endpoint.trim().to_string()
        };
        call_azure(&self.api_key, &endpoint, model, messages, max_tokens)
    }

    /// Delegates to the shared tiktoken-based estimator.
    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Call Azure `OpenAI` with a pre-built messages array (extraction path).
///
/// `messages` already carries the extraction system + user turns. Temperature is
/// resolved per-model (omitted for reasoning models, #1191) and `max_tokens` is
/// sent as `max_completion_tokens` (Azure rejects the deprecated `max_tokens`).
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation,
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport/parse errors, or
/// [`LlmError::EmptyResponse`] when Azure filters the response.
pub fn call_azure(
    api_key: &str,
    endpoint: &str,
    model: &str,
    messages: &[Value],
    max_tokens: u32,
) -> Result<LlmResponse, LlmError> {
    let url = chat_url(endpoint, model);
    graphify_security::validate_url(&url)?;

    let mut body = json!({
        "model": model,
        "messages": messages,
        "max_completion_tokens": max_tokens,
    });
    if let Some(t) = resolve_temperature(Some(0.0), model) {
        body["temperature"] = json!(t);
    }

    let value = send_azure_request(api_key, &url, &body)?;
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| {
            LlmError::EmptyResponse("Azure OpenAI returned empty or filtered response".to_string())
        })?;
    let message = choice
        .get("message")
        .filter(|m| !m.is_null())
        .ok_or_else(|| {
            LlmError::EmptyResponse("Azure OpenAI returned empty or filtered response".to_string())
        })?;
    let raw_content = message.get("content").and_then(Value::as_str);

    let parsed = parse_llm_json(raw_content.unwrap_or("{}"));
    let usage = value.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop")
        .to_string();
    if response_is_hollow(raw_content, &parsed) && finish_reason != "length" {
        eprintln!(
            "[graphify] azure returned a hollow response; treating as \
             truncation so adaptive retry can bisect the chunk."
        );
        finish_reason = "length".to_string();
    }

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
        uncovered_files: vec![],
        out_of_scope_dropped: 0,
    })
}

/// Plain-text Azure call for the LLM tiebreaker / labeling paths.
///
/// # Errors
/// Same as [`call_azure`].
pub fn call_azure_plain(
    api_key: &str,
    endpoint: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
    usage: Option<&crate::call::UsageSink>,
) -> Result<String, LlmError> {
    let url = chat_url(endpoint, model);
    graphify_security::validate_url(&url)?;
    let mut body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_completion_tokens": max_tokens,
    });
    if let Some(t) = resolve_temperature(Some(0.0), model) {
        body["temperature"] = json!(t);
    }
    let value = send_azure_request(api_key, &url, &body)?;
    if let Some(sink) = usage {
        let u = &value["usage"];
        sink.record(
            u["prompt_tokens"].as_u64().unwrap_or(0),
            u["completion_tokens"].as_u64().unwrap_or(0),
        );
    }
    Ok(value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string())
}

/// POST to the Azure chat-completions endpoint with the `api-key` header.
fn send_azure_request(api_key: &str, url: &str, body: &Value) -> Result<Value, LlmError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(api_timeout()))
        .build()
        .into();
    crate::openai_compat::send_json_with_retry(|| {
        agent
            .post(url)
            .header("api-key", api_key)
            .header("Content-Type", "application/json")
            .send_json(body)
    })
    .map_err(|e| LlmError::Http(e.to_string()))?
    .into_body()
    .read_json()
    .map_err(|e| LlmError::Parse(e.to_string()))
}
