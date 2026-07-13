//! Top-level plain-text LLM call dispatcher.
//!
//! Extracted from `lib.rs` to isolate `call_llm` — the function used by
//! `graphify-dedup` and other callers that need a raw text response rather
//! than a structured extraction fragment.

use crate::LlmError;
use crate::backends::{
    BACKENDS, backend_config, default_model_for_backend, format_backend_env_keys,
    get_backend_api_key,
};
use crate::{
    azure, bedrock, claude, claude_cli, deepseek, gemini, kimi, ollama, openai, openai_compat,
};

/// Thread-safe accumulator for LLM token usage (#1694).
///
/// Threaded through the community-labeling path so cluster-only mode reports the
/// real cost of otherwise-uninstrumented calls. Backends that do not return
/// usage contribute nothing (honest, not estimated). `Relaxed` ordering is
/// sufficient: only the final totals are read, after all recording completes.
#[derive(Debug, Default)]
pub struct UsageSink {
    input: std::sync::atomic::AtomicU64,
    output: std::sync::atomic::AtomicU64,
}

impl UsageSink {
    /// A zeroed accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one response's `input`/`output` token counts.
    pub fn record(&self, input: u64, output: u64) {
        self.input
            .fetch_add(input, std::sync::atomic::Ordering::Relaxed);
        self.output
            .fetch_add(output, std::sync::atomic::Ordering::Relaxed);
    }

    /// Total input tokens recorded.
    #[must_use]
    pub fn input(&self) -> u64 {
        self.input.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Total output tokens recorded.
    #[must_use]
    pub fn output(&self) -> u64 {
        self.output.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Send a plain-text `prompt` to the named `backend` and return the text reply.
///
/// Mirrors Python `_call_llm` at `llm.py:1719`. Unlike [`crate::extract_files_direct`],
/// this skips the extraction system prompt and JSON parsing — the caller receives
/// the raw model output as a `String`. Uses the backend's default model; callers
/// that need to override the model use [`call_llm_with_model`].
///
/// # Errors
/// - [`LlmError::UnknownBackend`] for unregistered backend names.
/// - [`LlmError::NoApiKey`] when no key is configured (except `bedrock`/`claude-cli`).
/// - Backend-specific HTTP / parse errors.
pub fn call_llm(prompt: &str, backend: &str, max_tokens: usize) -> Result<String, LlmError> {
    call_llm_with_model(prompt, backend, max_tokens, None)
}

/// [`call_llm`] with an optional model override.
///
/// When `model` is `Some(non_empty)`, it replaces the backend's default model
/// (`mdl = model or _default_model_for_backend(backend)` in Python). Used by
/// community labeling's `--model` flag (#b304331).
///
/// # Errors
/// Same as [`call_llm`].
pub fn call_llm_with_model(
    prompt: &str,
    backend: &str,
    max_tokens: usize,
    model: Option<&str>,
) -> Result<String, LlmError> {
    call_llm_with_model_usage(prompt, backend, max_tokens, model, None)
}

/// [`call_llm_with_model`] that accumulates each response's token usage into
/// `usage` when provided (#1694). Backends that do not return usage contribute
/// nothing. Used by the community-labeling path to total otherwise-untracked
/// cost; existing callers use the wrappers above and are unaffected.
///
/// # Errors
/// Same as [`call_llm`].
pub fn call_llm_with_model_usage(
    prompt: &str,
    backend: &str,
    max_tokens: usize,
    model: Option<&str>,
    usage: Option<&UsageSink>,
) -> Result<String, LlmError> {
    let max_tokens_u32 = u32::try_from(max_tokens).unwrap_or(u32::MAX);
    // Treat a blank `--model ""` as "no override", matching Python's
    // `model or default` (an empty string is falsy there).
    let model = model.map(str::trim).filter(|m| !m.is_empty());

    // Custom (non-built-in) provider: route through the OpenAI-compatible client
    // using the provider's base_url / model / env_key (#1084).
    if !crate::providers::is_builtin_backend(backend)
        && let Some(provider) = crate::providers::load_custom_providers().get(backend)
    {
        return call_custom_plain(provider, prompt, max_tokens_u32, model, usage);
    }

    // Validate the backend name; per-arm config (base URL, model) is resolved below.
    backend_config(backend).ok_or_else(|| {
        let available = BACKENDS
            .iter()
            .map(|b| b.name)
            .collect::<Vec<_>>()
            .join(", ");
        LlmError::UnknownBackend(backend.to_string(), available)
    })?;

    let key = get_backend_api_key(backend);

    // Resolve once and reuse below — `OLLAMA_BASE_URL` may be needed both
    // for the hard-block validation and for the actual `ollama` dispatch arm.
    let ollama_base_url = std::env::var("OLLAMA_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());

    // Ollama: hard-block link-local / cloud-metadata targets (F3) for *every*
    // ollama call, not only the no-key path. A non-empty OLLAMA_API_KEY must not
    // let a metadata `OLLAMA_BASE_URL` slip past the validator — and because
    // reaching a real local ollama needs GRAPHIFY_TEST_ALLOW_PRIVATE_IPS (which
    // also disarms the downstream SSRF guard), this F3 check is the only metadata
    // defence on the ollama path. graphify-py gates this behind `not key`
    // (llm.py:1170); fixing that gap is a deliberate divergence
    // (see [[feedback_python_bugs_are_not_requirements]]). The hard-block is
    // unconditional; the LAN warning stays on the no-key path to avoid a
    // spurious warning when the user has explicitly configured a key.
    let key = if backend == "ollama" {
        ollama::validate_ollama_base_url(&ollama_base_url, key.is_empty())?;
        // Accept a missing key, using the "ollama" sentinel (Ollama ignores it).
        if key.is_empty() {
            "ollama".to_string()
        } else {
            key
        }
    } else {
        key
    };

    if key.is_empty() && backend != "bedrock" && backend != "claude-cli" {
        return Err(LlmError::NoApiKey(format!(
            "No API key for backend '{backend}'. \
             Set {} or pass api_key=.",
            format_backend_env_keys(backend)
        )));
    }

    let resolved_default = default_model_for_backend(backend);
    let mdl = model.unwrap_or(resolved_default.as_ref());

    match backend {
        "claude" => call_claude_plain(prompt, mdl, max_tokens_u32, &key, usage),
        "claude-cli" => {
            claude_cli::call_claude_cli_plain_with_model(prompt, max_tokens_u32, model, usage)
        }
        "bedrock" => {
            let region = bedrock::resolve_region();
            bedrock::call_bedrock_plain(mdl, &region, prompt, max_tokens_u32, usage)
        }
        "kimi" => kimi::call_plain_openai_compat(&kimi::PlainOpenAiRequest {
            base_url: &kimi::base_url(),
            api_key: &key,
            model: mdl,
            prompt,
            temperature: Some(0.0),
            reasoning_effort: None,
            disable_thinking: true,
            extra_body: None,
            max_tokens: max_tokens_u32,
            usage,
        }),
        "gemini" => gemini::call_gemini_plain(&key, mdl, prompt, max_tokens_u32, usage),
        "openai" => openai::call_openai_plain(&key, mdl, prompt, max_tokens_u32, usage),
        "deepseek" => deepseek::call_deepseek_plain(&key, mdl, prompt, max_tokens_u32, usage),
        "ollama" => {
            ollama::call_ollama_plain(&key, &ollama_base_url, mdl, prompt, max_tokens_u32, usage)
        }
        "azure" => {
            // Resolve the deployment from the environment when no override is
            // given, then require AZURE_OPENAI_ENDPOINT.
            let azure_mdl = model.map_or_else(azure::resolve_model, str::to_string);
            let endpoint = azure::resolve_endpoint()?;
            azure::call_azure_plain(&key, &endpoint, &azure_mdl, prompt, max_tokens_u32, usage)
        }
        _ => unreachable!("backend_config already validated backend name"),
    }
}

/// Anthropic `/v1/messages` plain call, recording usage into `usage` (#1694).
fn call_claude_plain(
    prompt: &str,
    model: &str,
    max_tokens: u32,
    api_key: &str,
    usage: Option<&UsageSink>,
) -> Result<String, LlmError> {
    let timeout = openai_compat::api_timeout();
    let claude_base = claude::base_url();
    graphify_security::validate_url(&claude_base)?;
    let endpoint = format!("{claude_base}/v1/messages");
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{"role": "user", "content": prompt}],
    });
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    let http_resp = openai_compat::send_json_with_retry(|| {
        agent
            .post(&endpoint)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .send_json(&body)
    })
    .map_err(|e| LlmError::Http(e.to_string()))?;
    // Deserialize just enough to extract the text and usage.
    let val: serde_json::Value = http_resp
        .into_body()
        .read_json()
        .map_err(|e| LlmError::Parse(e.to_string()))?;
    let content = val["content"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v["text"].as_str())
        .unwrap_or("")
        .to_string();
    if let Some(sink) = usage {
        let u = &val["usage"];
        sink.record(
            u["input_tokens"].as_u64().unwrap_or(0),
            u["output_tokens"].as_u64().unwrap_or(0),
        );
    }
    Ok(content)
}

/// Plain-text call against a custom OpenAI-compatible provider (#1084).
///
/// `model` overrides the provider's `default_model` when `Some` (#b304331).
fn call_custom_plain(
    provider: &crate::providers::CustomProvider,
    prompt: &str,
    max_tokens: u32,
    model: Option<&str>,
    usage: Option<&UsageSink>,
) -> Result<String, LlmError> {
    let key = std::env::var(&provider.env_key).unwrap_or_default();
    if key.is_empty() {
        return Err(LlmError::NoApiKey(format!(
            "No API key for backend '{}'. Set {}.",
            provider.name, provider.env_key
        )));
    }
    kimi::call_plain_openai_compat(&kimi::PlainOpenAiRequest {
        base_url: &provider.base_url,
        api_key: &key,
        model: model.unwrap_or(&provider.default_model),
        prompt,
        temperature: Some(provider.temperature),
        reasoning_effort: None,
        disable_thinking: false,
        extra_body: provider.extra_body.as_ref(),
        max_tokens,
        usage,
    })
}
