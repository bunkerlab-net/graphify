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

    // Ollama: use sentinel "ollama" key when none configured.
    let key = if key.is_empty() && backend == "ollama" {
        let ollama_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        ollama::validate_ollama_base_url(&ollama_url);
        eprintln!(
            "[graphify] WARNING: ollama backend selected with no OLLAMA_API_KEY set; \
             sending corpus to {ollama_url}. Set OLLAMA_API_KEY (any non-empty value) \
             to suppress this warning."
        );
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

    let mdl = model.filter(|s| !s.is_empty()).unwrap_or(cfg.default_model);
    let user_msg = read_files(files, root);
    // `resolve_max_tokens` applies the `GRAPHIFY_MAX_OUTPUT_TOKENS` env var
    // override uniformly across every backend (parity fix from
    // graphify-py 06a9b72 — env var was previously silently ignored on the
    // OpenAI-compatible path because the cfg dict's hardcoded value shadowed
    // the resolved value).
    let max_out = openai_compat::resolve_max_tokens(cfg.default_max_tokens);

    match backend {
        "claude" => {
            let msgs = vec![serde_json::json!({"role": "user", "content": user_msg})];
            claude::call_claude(&key, mdl, &msgs, max_out)
        }
        "claude-cli" => {
            let runner = claude_cli::RealClaudeRunner;
            claude_cli::call_claude_cli_with_runner(&runner, &user_msg, max_out)
        }
        "bedrock" => {
            let region = bedrock::resolve_region();
            let msgs = vec![serde_json::json!({"role": "user", "content": [{"text": user_msg}]})];
            bedrock::call_bedrock(mdl, &region, &msgs, max_out)
        }
        "kimi" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            kimi::call_kimi(&key, mdl, &msgs, max_out)
        }
        "gemini" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            gemini::call_gemini(&key, mdl, &msgs, max_out)
        }
        "openai" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            openai::call_openai(&key, mdl, &msgs, max_out)
        }
        "deepseek" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            deepseek::call_deepseek(&key, mdl, &msgs, max_out)
        }
        "ollama" => {
            let base_url = std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
            let msgs = openai_compat::extraction_messages(&user_msg);
            ollama::call_ollama(&key, &base_url, mdl, &msgs, max_out, &user_msg)
        }
        _ => unreachable!("backend_config already validated backend name"),
    }
}
