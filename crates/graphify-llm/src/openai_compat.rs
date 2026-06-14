//! Shared HTTP helper for `OpenAI`-compatible backends (Kimi, Gemini, Ollama).
//!
//! Uses `ureq` directly (not `graphify_security::safe_fetch`) because LLM
//! responses can exceed 10 MB (the `safe_fetch` cap). URL validation is
//! performed with `graphify_security::validate_url` before each request,
//! preserving the same SSRF posture.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::tokenizer::CHARS_PER_TOKEN;
use crate::{LLM_JSON_MAX_BYTES, LlmError, LlmResponse, parse_llm_json, response_is_hollow};

/// Config passed in from a backend for one API call.
pub struct OpenAiRequest<'a> {
    /// Base URL of the OpenAI-compatible endpoint (e.g. `https://api.openai.com/v1`).
    pub base_url: &'a str,
    /// Bearer token sent in the `Authorization` header.
    pub api_key: &'a str,
    /// Model identifier forwarded verbatim to the API.
    pub model: &'a str,
    /// Pre-built messages array, including any system prompt.
    pub messages: Vec<Value>,
    /// Sampling temperature; `None` omits the field from the request body.
    pub temperature: Option<f64>,
    /// Reasoning effort hint (e.g. `"low"`); `None` omits the field.
    pub reasoning_effort: Option<&'a str>,
    /// Maximum tokens the model may generate in its reply.
    pub max_completion_tokens: u32,
    /// If `true`, inject Kimi's `thinking: disabled` extra body.
    pub disable_thinking: bool,
    /// Custom-provider `extra_body` passthrough (#7477b46). When `Some`, it owns
    /// the request's `extra_body` and overrides both [`Self::disable_thinking`]
    /// and [`Self::ollama_options`] — the provider has chosen its request shape.
    pub custom_extra_body: Option<&'a Value>,
    /// Ollama-specific options: `num_ctx`, `keep_alive`.
    pub ollama_options: Option<OllamaOptions>,
    /// Backend name for diagnostic messages.
    pub backend_name: &'a str,
    /// Timeout for the HTTP request.
    pub timeout: Duration,
}

/// Ollama-specific extra-body options.
pub struct OllamaOptions {
    /// Context window size passed as `options.num_ctx` in the Ollama request body.
    pub num_ctx: u32,
    /// How long to keep the model loaded between requests (e.g. `"30m"`).
    pub keep_alive: String,
}

#[derive(Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Deserialize)]
struct OaiChoice {
    message: Option<OaiMessage>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OaiMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OaiUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

/// Call an `OpenAI`-compatible endpoint and return an [`LlmResponse`].
///
/// Validates the URL (SSRF guard) before each request.
/// Accepts a pre-built messages array so callers can inject system prompts.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] / [`LlmError::EmptyResponse`] on
/// transport or response errors.
pub fn call_openai_compat(req: &OpenAiRequest<'_>) -> Result<LlmResponse, LlmError> {
    // SSRF guard — validate URL before making any network call.
    graphify_security::validate_url(req.base_url)?;

    let body = build_chat_request_body(req);
    let resp_body = send_chat_request(req, &body)?;
    let choice = resp_body
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| LlmError::EmptyResponse("no choices in response".to_string()))?;

    let raw_content = choice.message.and_then(|m| m.content);
    let finish_reason_raw = choice.finish_reason.unwrap_or_else(|| "stop".to_string());
    let content_str = raw_content.as_deref().unwrap_or("");
    let input_tokens = resp_body
        .usage
        .as_ref()
        .and_then(|u| u.prompt_tokens)
        .unwrap_or(0);
    let output_tokens = resp_body
        .usage
        .as_ref()
        .and_then(|u| u.completion_tokens)
        .unwrap_or(0);

    let mut parsed = parse_llm_json(content_str);
    let finish_reason = resolve_finish_reason(
        req,
        raw_content.as_deref(),
        content_str,
        output_tokens,
        &parsed,
        &finish_reason_raw,
    );
    maybe_warn_low_token_ollama(req, output_tokens);

    parsed["input_tokens"] = json!(input_tokens);
    parsed["output_tokens"] = json!(output_tokens);
    parsed["model"] = json!(req.model);
    parsed["finish_reason"] = json!(finish_reason.as_str());

    Ok(LlmResponse {
        nodes: parsed["nodes"].as_array().cloned().unwrap_or_default(),
        edges: parsed["edges"].as_array().cloned().unwrap_or_default(),
        hyperedges: parsed["hyperedges"].as_array().cloned().unwrap_or_default(),
        input_tokens,
        output_tokens,
        model: req.model.to_string(),
        finish_reason,
        elapsed_seconds: 0.0,
        failed_chunk_indices: vec![],
    })
}

/// Build the OpenAI-compatible chat-completions JSON body from the request bundle.
/// Model-name fragments for `OpenAI`-compatible "reasoning" models that reject
/// an explicit temperature (the API returns HTTP 400 for any value, including
/// 0). Covers the o1/o3/o4 series and the gpt-5 family (#1191). Matched
/// case-insensitively against the resolved model id, ignoring a provider prefix
/// some gateways prepend (e.g. `openai/o3-mini`).
#[must_use]
pub fn model_requires_default_temperature(model: &str) -> bool {
    let m = model.to_lowercase();
    let base = m.rsplit('/').next().unwrap_or(m.as_str());
    if base.starts_with("gpt-5") {
        return true;
    }
    ["o1", "o3", "o4"]
        .iter()
        .any(|fam| base == *fam || base.starts_with(&format!("{fam}-")))
}

/// Resolve the temperature to send, honouring `GRAPHIFY_LLM_TEMPERATURE`
/// (#1191). Precedence:
/// 1. `GRAPHIFY_LLM_TEMPERATURE`, if set: a number is used verbatim; the literal
///    `none`/`omit`/`default` (case-insensitive) omits the parameter (`None`).
/// 2. Otherwise reasoning models (o1/o3/o4/gpt-5) get `None` — the API rejects
///    any explicit temperature.
/// 3. Otherwise the backend default.
///
/// Returns `None` when the temperature field should be omitted entirely.
#[must_use]
pub fn resolve_temperature(default: Option<f64>, model: &str) -> Option<f64> {
    let raw = std::env::var("GRAPHIFY_LLM_TEMPERATURE").unwrap_or_default();
    let raw = raw.trim();
    if !raw.is_empty() {
        let lower = raw.to_lowercase();
        if lower == "none" || lower == "omit" || lower == "default" {
            return None;
        }
        if let Ok(v) = raw.parse::<f64>() {
            return Some(v);
        }
        // Unparseable override falls through to the model/default logic.
    }
    if model_requires_default_temperature(model) {
        return None;
    }
    default
}

fn build_chat_request_body(req: &OpenAiRequest<'_>) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages,
        "max_completion_tokens": req.max_completion_tokens,
    });
    if let Some(t) = resolve_temperature(req.temperature, req.model) {
        body["temperature"] = json!(t);
    }
    if let Some(re) = req.reasoning_effort {
        body["reasoning_effort"] = json!(re);
    }
    // A custom provider's explicit extra_body owns the request shape and wins
    // over both the moonshot `thinking: disabled` default and the Ollama
    // num_ctx auto-derive (#7477b46).
    if let Some(custom) = req.custom_extra_body {
        body["extra_body"] = custom.clone();
        return body;
    }
    // Build `extra_body` incrementally so disable_thinking and ollama
    // options can coexist (assigning each one separately would overwrite
    // the prior value).
    let mut extra_body = serde_json::Map::new();
    if req.disable_thinking {
        // Kimi-k2.6 — disable thinking so content isn't empty.
        extra_body.insert("thinking".to_string(), json!({"type": "disabled"}));
    }
    if let Some(opts) = &req.ollama_options {
        extra_body.insert("options".to_string(), json!({"num_ctx": opts.num_ctx}));
        extra_body.insert("keep_alive".to_string(), json!(opts.keep_alive));
    }
    if !extra_body.is_empty() {
        body["extra_body"] = Value::Object(extra_body);
    }
    body
}

/// POST the chat-completions request and parse the response body.
fn send_chat_request(req: &OpenAiRequest<'_>, body: &Value) -> Result<OaiResponse, LlmError> {
    let endpoint = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(req.timeout))
        .build()
        .into();
    let http_resp = agent
        .post(&endpoint)
        .header("Authorization", &format!("Bearer {}", req.api_key))
        .header("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| LlmError::Http(e.to_string()))?;
    http_resp
        .into_body()
        .read_json()
        .map_err(|e| LlmError::Parse(e.to_string()))
}

/// Re-label `finish_reason="length"` when the response is structurally hollow
/// so the adaptive-retry layer bisects the chunk.
fn resolve_finish_reason(
    req: &OpenAiRequest<'_>,
    raw_content: Option<&str>,
    content_str: &str,
    output_tokens: u64,
    parsed: &Value,
    finish_reason_raw: &str,
) -> String {
    let mut finish_reason = if finish_reason_raw == "length" {
        "length".to_string()
    } else {
        "stop".to_string()
    };
    if response_is_hollow(raw_content, parsed) && finish_reason != "length" {
        let content_desc = if content_str.trim().is_empty() {
            "empty"
        } else {
            "no nodes/edges"
        };
        eprintln!(
            "[graphify] {} returned a hollow response \
             (content={content_desc}, output_tokens={output_tokens}); \
             treating as truncation so adaptive retry can bisect the chunk.",
            req.backend_name
        );
        finish_reason = "length".to_string();
    }
    finish_reason
}

/// Surface a hint about VRAM / model size when Ollama responses come back tiny.
fn maybe_warn_low_token_ollama(req: &OpenAiRequest<'_>, output_tokens: u64) {
    if output_tokens < 50 && req.backend_name == "ollama" {
        eprintln!(
            "[graphify] warning: ollama returned very few tokens — likely causes: \
             (1) VRAM pressure: check `nvidia-smi` and reduce chunk size with \
             --token-budget (e.g. --token-budget 4096) or set \
             GRAPHIFY_OLLAMA_NUM_CTX to a smaller value; \
             (2) model too small for JSON instruction following — \
             try a larger model with --model (e.g. --model qwen2.5-coder:14b)."
        );
    }
}

/// Compute Ollama's `num_ctx` from the user message length.
///
/// Mirrors the Python derivation: `estimated_input + max_completion_tokens + 2000`,
/// clamped to `[8192, 131072]`.
#[must_use]
pub fn derive_ollama_num_ctx(user_message: &str, max_completion_tokens: u32) -> u32 {
    let estimated_input = user_message.len() / CHARS_PER_TOKEN + 400;
    let auto = (estimated_input + max_completion_tokens as usize + 2000).clamp(8_192, 131_072);
    u32::try_from(auto).unwrap_or(131_072)
}

/// Build the standard extraction messages array (system + user).
#[must_use]
pub fn extraction_messages(user_message: &str) -> Vec<Value> {
    extraction_messages_for(user_message, false)
}

/// Build the extraction messages array, selecting the deep-mode system prompt
/// when `deep` is set.
#[must_use]
pub fn extraction_messages_for(user_message: &str, deep: bool) -> Vec<Value> {
    let system = crate::constants::extraction_system(deep);
    vec![
        json!({"role": "system", "content": system.as_ref()}),
        json!({"role": "user", "content": user_message}),
    ]
}

/// Build a plain-text (no system prompt) messages array.
#[must_use]
pub fn plain_messages(user_message: &str) -> Vec<Value> {
    vec![json!({"role": "user", "content": user_message})]
}

/// Return the configured HTTP timeout (seconds).
///
/// Reads `GRAPHIFY_API_TIMEOUT`; defaults to 600 s.
#[must_use]
pub fn api_timeout() -> Duration {
    let raw = std::env::var("GRAPHIFY_API_TIMEOUT").unwrap_or_default();
    let raw = raw.trim();
    if !raw.is_empty()
        && let Ok(v) = raw.parse::<f64>()
        && v > 0.0
    {
        return Duration::from_secs_f64(v);
    }
    Duration::from_mins(10)
}

/// Return the configured max-output-tokens override.
///
/// Reads `GRAPHIFY_MAX_OUTPUT_TOKENS`; returns `default` if not set or invalid.
#[must_use]
pub fn resolve_max_tokens(default: u32) -> u32 {
    let raw = std::env::var("GRAPHIFY_MAX_OUTPUT_TOKENS").unwrap_or_default();
    let raw = raw.trim();
    if !raw.is_empty()
        && let Ok(v) = raw.parse::<u32>()
        && v > 0
    {
        return v;
    }
    default
}

/// Cap raw response at [`LLM_JSON_MAX_BYTES`] then parse the JSON.
///
/// Returns an empty fragment `{nodes:[], edges:[], hyperedges:[]}` on failure.
#[must_use]
pub fn safe_parse_response(raw: &str) -> Value {
    if raw.len() > LLM_JSON_MAX_BYTES {
        eprintln!(
            "[graphify] LLM response exceeds {LLM_JSON_MAX_BYTES} bytes \
             ({} bytes); refusing to parse and dropping chunk.",
            raw.len()
        );
        return crate::empty_fragment();
    }
    parse_llm_json(raw)
}
