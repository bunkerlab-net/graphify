//! Direct file extraction via a named backend.
//!
//! Extracted from `lib.rs` to isolate `extract_files_direct` — the function
//! that reads files, validates keys, and dispatches to the correct backend
//! `call_*` function.

use std::path::{Path, PathBuf};

use crate::backends::{BACKENDS, backend_config, format_backend_env_keys, get_backend_api_key};
use crate::read::read_files;
use crate::{LlmError, LlmResponse};
use crate::{bedrock, claude, claude_cli, deepseek, gemini, kimi, ollama, openai, openai_compat};

/// Extract semantic nodes/edges from a list of files using the given backend.
///
/// # Errors
/// - [`LlmError::UnknownBackend`] for unregistered backend names.
/// - [`LlmError::NoApiKey`] when no key is configured (except `bedrock`/`claude-cli`).
/// - Backend-specific HTTP / parse errors.
pub fn extract_files_direct(
    files: &[PathBuf],
    backend: &str,
    api_key: Option<&str>,
    model: Option<&str>,
    root: &Path,
) -> Result<LlmResponse, LlmError> {
    extract_files_direct_mode(files, backend, api_key, model, root, false)
}

/// Same as [`extract_files_direct`], selecting the deep-mode extraction system
/// prompt when `deep_mode` is set (the CLI's `--mode deep`).
///
/// # Errors
/// Same as [`extract_files_direct`].
pub fn extract_files_direct_mode(
    files: &[PathBuf],
    backend: &str,
    api_key: Option<&str>,
    model: Option<&str>,
    root: &Path,
    deep_mode: bool,
) -> Result<LlmResponse, LlmError> {
    // Custom (non-built-in) provider: extract via the OpenAI-compatible client
    // using the provider's base_url / model / env_key (#1084).
    if !crate::providers::is_builtin_backend(backend)
        && let Some(provider) = crate::providers::load_custom_providers().get(backend)
    {
        return extract_custom(provider, files, api_key, model, root, deep_mode);
    }

    let cfg = backend_config(backend).ok_or_else(|| {
        let available = BACKENDS
            .iter()
            .map(|b| b.name)
            .collect::<Vec<_>>()
            .join(", ");
        LlmError::UnknownBackend(backend.to_string(), available)
    })?;

    let key = api_key
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| get_backend_api_key(backend));

    // Ollama: hard-block link-local / cloud-metadata targets (F3) for *every*
    // ollama call, not only the no-key path — a non-empty OLLAMA_API_KEY must
    // not let a metadata `OLLAMA_BASE_URL` slip past the validator (graphify-py
    // gates this behind `not key` at llm.py:751; fixing that gap is a deliberate
    // divergence, see [[feedback_python_bugs_are_not_requirements]]). The no-key
    // warning + "ollama" sentinel stay on the empty-key path.
    let key = if backend == "ollama" {
        let ollama_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        ollama::validate_ollama_base_url(&ollama_url, key.is_empty())?;
        if key.is_empty() {
            eprintln!(
                "[graphify] WARNING: ollama backend selected with no OLLAMA_API_KEY set; \
                 sending corpus to {ollama_url}. Set OLLAMA_API_KEY (any non-empty value) \
                 to suppress this warning."
            );
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

    let mdl = model.filter(|s| !s.is_empty()).unwrap_or(cfg.default_model);
    let user_msg = read_files(files, root);
    // `resolve_max_tokens` applies the `GRAPHIFY_MAX_OUTPUT_TOKENS` env var
    // override uniformly across every backend (parity fix from
    // graphify-py 06a9b72 — env var was previously silently ignored on the
    // OpenAI-compatible path because the cfg dict's hardcoded value shadowed
    // the resolved value).
    let max_out = openai_compat::resolve_max_tokens(cfg.default_max_tokens);
    // Deep mode appends an INFERRED-edge suffix to the extraction system prompt.
    let system = crate::constants::extraction_system(deep_mode);

    match backend {
        "claude" => {
            let msgs = vec![serde_json::json!({"role": "user", "content": user_msg})];
            claude::call_claude_with_system(&key, mdl, &msgs, max_out, system.as_ref())
        }
        "claude-cli" => {
            let runner = claude_cli::RealClaudeRunner;
            claude_cli::call_claude_cli_with_runner_system(
                &runner,
                &user_msg,
                max_out,
                system.as_ref(),
            )
        }
        "bedrock" => {
            let region = bedrock::resolve_region();
            let msgs = vec![serde_json::json!({"role": "user", "content": [{"text": user_msg}]})];
            bedrock::call_bedrock_with_system(mdl, &region, &msgs, max_out, system.as_ref())
        }
        "kimi" => {
            let msgs = openai_compat::extraction_messages_for(&user_msg, deep_mode);
            kimi::call_kimi(&key, mdl, &msgs, max_out)
        }
        "gemini" => {
            let msgs = openai_compat::extraction_messages_for(&user_msg, deep_mode);
            gemini::call_gemini(&key, mdl, &msgs, max_out)
        }
        "openai" => {
            let msgs = openai_compat::extraction_messages_for(&user_msg, deep_mode);
            openai::call_openai(&key, mdl, &msgs, max_out)
        }
        "deepseek" => {
            let msgs = openai_compat::extraction_messages_for(&user_msg, deep_mode);
            deepseek::call_deepseek(&key, mdl, &msgs, max_out)
        }
        "ollama" => {
            let base_url = std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
            let msgs = openai_compat::extraction_messages_for(&user_msg, deep_mode);
            ollama::call_ollama(&key, &base_url, mdl, &msgs, max_out, &user_msg)
        }
        _ => unreachable!("backend_config already validated backend name"),
    }
}

/// Extract via a custom OpenAI-compatible provider (#1084). Honors an explicit
/// `api_key`/`model`, else falls back to the provider's `env_key`/`default_model`.
fn extract_custom(
    provider: &crate::providers::CustomProvider,
    files: &[PathBuf],
    api_key: Option<&str>,
    model: Option<&str>,
    root: &Path,
    deep_mode: bool,
) -> Result<LlmResponse, LlmError> {
    let key = api_key
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::var(&provider.env_key).unwrap_or_default());
    if key.is_empty() {
        return Err(LlmError::NoApiKey(format!(
            "No API key for backend '{}'. Set {} or pass api_key=.",
            provider.name, provider.env_key
        )));
    }
    let mdl = model
        .filter(|s| !s.is_empty())
        .unwrap_or(&provider.default_model);
    let user_msg = read_files(files, root);
    // Honour the provider's configured `max_completion_tokens` (default 8192),
    // then apply the `GRAPHIFY_MAX_OUTPUT_TOKENS` override — mirroring Python's
    // `_resolve_max_tokens(cfg.get("max_completion_tokens", 8192))` (llm.py:720).
    let max_out = openai_compat::resolve_max_tokens(provider.max_completion_tokens);
    openai_compat::call_openai_compat(&openai_compat::OpenAiRequest {
        base_url: &provider.base_url,
        api_key: &key,
        model: mdl,
        messages: openai_compat::extraction_messages_for(&user_msg, deep_mode),
        temperature: Some(provider.temperature),
        reasoning_effort: None,
        max_completion_tokens: max_out,
        disable_thinking: false,
        ollama_options: None,
        backend_name: &provider.name,
        timeout: openai_compat::api_timeout(),
    })
}
