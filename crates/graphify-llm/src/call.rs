//! Top-level plain-text LLM call dispatcher.
//!
//! Extracted from `lib.rs` to isolate `call_llm` — the function used by
//! `graphify-dedup` and other callers that need a raw text response rather
//! than a structured extraction fragment.

use crate::LlmError;
use crate::backends::{BACKENDS, backend_config, format_backend_env_keys, get_backend_api_key};
use crate::{bedrock, claude_cli, deepseek, gemini, kimi, ollama, openai, openai_compat};

/// Send a plain-text `prompt` to the named `backend` and return the text reply.
///
/// Mirrors Python `_call_llm` at `llm.py:948`. Unlike [`crate::extract_files_direct`],
/// this skips the extraction system prompt and JSON parsing — the caller receives
/// the raw model output as a `String`.
///
/// # Errors
/// - [`LlmError::UnknownBackend`] for unregistered backend names.
/// - [`LlmError::NoApiKey`] when no key is configured (except `bedrock`/`claude-cli`).
/// - Backend-specific HTTP / parse errors.
pub fn call_llm(prompt: &str, backend: &str, max_tokens: usize) -> Result<String, LlmError> {
    let max_tokens_u32 = u32::try_from(max_tokens).unwrap_or(u32::MAX);

    let cfg = backend_config(backend).ok_or_else(|| {
        let available = BACKENDS
            .iter()
            .map(|b| b.name)
            .collect::<Vec<_>>()
            .join(", ");
        LlmError::UnknownBackend(backend.to_string(), available)
    })?;

    let key = get_backend_api_key(backend);

    // Ollama: accept missing key, use sentinel.
    let key = if key.is_empty() && backend == "ollama" {
        let ollama_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        ollama::validate_ollama_base_url(&ollama_url);
        "ollama".to_string()
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

    let mdl = cfg.default_model;

    match backend {
        "claude" => {
            // Call Anthropic API without the extraction system prompt.
            let timeout = openai_compat::api_timeout();
            graphify_security::validate_url("https://api.anthropic.com")?;
            let endpoint = "https://api.anthropic.com/v1/messages";
            let body = serde_json::json!({
                "model": mdl,
                "max_tokens": max_tokens_u32,
                "messages": [{"role": "user", "content": prompt}],
            });
            let agent = ureq::AgentBuilder::new().timeout(timeout).build();
            let http_resp = agent
                .post(endpoint)
                .set("x-api-key", &key)
                .set("anthropic-version", "2023-06-01")
                .set("Content-Type", "application/json")
                .send_json(&body)
                .map_err(|e| LlmError::Http(e.to_string()))?;
            // Deserialize just enough to extract the text.
            let val: serde_json::Value = http_resp
                .into_json()
                .map_err(|e| LlmError::Parse(e.to_string()))?;
            Ok(val["content"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v["text"].as_str())
                .unwrap_or("")
                .to_string())
        }
        "claude-cli" => claude_cli::call_claude_cli_plain(prompt, max_tokens_u32),
        "bedrock" => {
            let region = bedrock::resolve_region();
            bedrock::call_bedrock_plain(mdl, &region, prompt, max_tokens_u32)
        }
        "kimi" => kimi::call_plain_openai_compat(
            "https://api.moonshot.ai/v1",
            &key,
            mdl,
            prompt,
            Some(0.0),
            None,
            true,
            max_tokens_u32,
        ),
        "gemini" => gemini::call_gemini_plain(&key, mdl, prompt, max_tokens_u32),
        "openai" => openai::call_openai_plain(&key, mdl, prompt, max_tokens_u32),
        "deepseek" => deepseek::call_deepseek_plain(&key, mdl, prompt, max_tokens_u32),
        "ollama" => {
            let base_url = std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
            ollama::call_ollama_plain(&key, &base_url, mdl, prompt, max_tokens_u32)
        }
        _ => unreachable!("backend_config already validated backend name"),
    }
}
