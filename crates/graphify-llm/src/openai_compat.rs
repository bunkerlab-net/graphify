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
use crate::{
    EXTRACTION_SYSTEM, LLM_JSON_MAX_BYTES, LlmError, LlmResponse, parse_llm_json,
    response_is_hollow,
};

/// Config passed in from a backend for one API call.
pub struct OpenAiRequest<'a> {
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    pub messages: Vec<Value>,
    pub temperature: Option<f64>,
    pub reasoning_effort: Option<&'a str>,
    pub max_completion_tokens: u32,
    /// If `true`, inject Kimi's `thinking: disabled` extra body.
    pub disable_thinking: bool,
    /// Ollama-specific options: `num_ctx`, `keep_alive`.
    pub ollama_options: Option<OllamaOptions>,
    /// Backend name for diagnostic messages.
    pub backend_name: &'a str,
    /// Timeout for the HTTP request.
    pub timeout: Duration,
}

/// Ollama-specific extra-body options.
pub struct OllamaOptions {
    pub num_ctx: u32,
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
#[allow(clippy::too_many_lines)]
pub fn call_openai_compat(req: &OpenAiRequest<'_>) -> Result<LlmResponse, LlmError> {
    // SSRF guard — validate URL before making any network call.
    graphify_security::validate_url(req.base_url)?;

    let mut body = json!({
        "model": req.model,
        "messages": req.messages,
        "max_completion_tokens": req.max_completion_tokens,
    });

    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(re) = req.reasoning_effort {
        body["reasoning_effort"] = json!(re);
    }

    // Kimi-k2.6 — disable thinking so content isn't empty.
    if req.disable_thinking {
        body["extra_body"] = json!({"thinking": {"type": "disabled"}});
    }

    // Ollama — inject num_ctx + keep_alive.
    if let Some(opts) = &req.ollama_options {
        body["extra_body"] = json!({
            "options": {"num_ctx": opts.num_ctx},
            "keep_alive": opts.keep_alive,
        });
    }

    let endpoint = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(req.timeout))
        .build()
        .into();

    let http_resp = agent
        .post(&endpoint)
        .header("Authorization", &format!("Bearer {}", req.api_key))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| LlmError::Http(e.to_string()))?;

    let resp_body: OaiResponse = http_resp
        .into_body()
        .read_json()
        .map_err(|e| LlmError::Parse(e.to_string()))?;

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
    let mut finish_reason = if finish_reason_raw == "length" {
        "length".to_string()
    } else {
        "stop".to_string()
    };

    // Hollow-response detection: re-label as "length" so adaptive retry bisects.
    if response_is_hollow(raw_content.as_deref(), &parsed) && finish_reason != "length" {
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
    vec![
        json!({"role": "system", "content": EXTRACTION_SYSTEM}),
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
    Duration::from_secs(600)
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
