//! Backend registry, auto-detection, and API-key helpers.
//!
//! Extracted from `lib.rs` to isolate the `BACKENDS` const, `detect_backend`,
//! `get_backend_api_key`, and `format_backend_env_keys` — the static metadata
//! and environment-key resolution layer shared by all call sites.

use crate::{
    LlmBackend, LlmError, bedrock, claude, claude_cli, deepseek, gemini, kimi, ollama, openai,
};

/// Pricing entry (USD per 1M tokens).
#[derive(Debug, Clone, Copy)]
pub struct Pricing {
    /// Cost per 1 million input (prompt) tokens, in USD.
    pub input: f64,
    /// Cost per 1 million output (completion) tokens, in USD.
    pub output: f64,
}

/// Static backend metadata (mirrors Python `BACKENDS` dict).
#[derive(Debug, Clone)]
pub struct BackendConfig {
    /// Short identifier used to select the backend (e.g. `"claude"`, `"openai"`).
    pub name: &'static str,
    /// Model string used when the caller does not supply an explicit override.
    pub default_model: &'static str,
    /// Published pricing for cost estimation (may be stale — update as needed).
    pub pricing: Pricing,
    /// Maximum output tokens requested when the caller does not override.
    pub default_max_tokens: u32,
}

/// All registered backends.
pub const BACKENDS: &[BackendConfig] = &[
    BackendConfig {
        name: "claude",
        default_model: claude::DEFAULT_MODEL,
        pricing: Pricing {
            input: 3.0,
            output: 15.0,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "kimi",
        default_model: kimi::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.74,
            output: 4.66,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "gemini",
        default_model: gemini::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.50,
            output: 3.00,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "openai",
        default_model: openai::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.40,
            output: 1.60,
        },
        default_max_tokens: 8_192,
    },
    BackendConfig {
        name: "deepseek",
        default_model: deepseek::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.14,
            output: 0.28,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "ollama",
        default_model: ollama::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.0,
            output: 0.0,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "bedrock",
        default_model: bedrock::DEFAULT_MODEL,
        pricing: Pricing {
            input: 3.0,
            output: 15.0,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "claude-cli",
        default_model: "claude-code-plan",
        pricing: Pricing {
            input: 0.0,
            output: 0.0,
        },
        default_max_tokens: 16_384,
    },
];

/// Look up a backend config by name.
#[must_use]
pub fn backend_config(name: &str) -> Option<&'static BackendConfig> {
    BACKENDS.iter().find(|b| b.name == name)
}

/// Construct a boxed [`LlmBackend`] by name.
///
/// # Errors
/// Returns [`LlmError::UnknownBackend`] if `name` is not registered.
pub fn router(name: &str) -> Result<Box<dyn LlmBackend>, LlmError> {
    match name {
        "claude" => Ok(Box::new(claude::ClaudeBackend::from_env())),
        "kimi" => Ok(Box::new(kimi::KimiBackend::from_env())),
        "gemini" => Ok(Box::new(gemini::GeminiBackend::from_env())),
        "openai" => Ok(Box::new(openai::OpenAiBackend::from_env())),
        "deepseek" => Ok(Box::new(deepseek::DeepSeekBackend::from_env())),
        "ollama" => Ok(Box::new(ollama::OllamaBackend::from_env())),
        "bedrock" => Ok(Box::new(bedrock::BedrockBackend::from_env())),
        "claude-cli" => Ok(Box::new(claude_cli::ClaudeCliBackend::new())),
        other => {
            let available = BACKENDS
                .iter()
                .map(|b| b.name)
                .collect::<Vec<_>>()
                .join(", ");
            Err(LlmError::UnknownBackend(other.to_string(), available))
        }
    }
}

/// Return the first available API key for the named backend (or empty string).
#[must_use]
pub fn get_backend_api_key(backend: &str) -> String {
    match backend {
        "gemini" => gemini::get_api_key(),
        "kimi" => std::env::var(kimi::ENV_KEY).unwrap_or_default(),
        "claude" => std::env::var(claude::ENV_KEY).unwrap_or_default(),
        "openai" => std::env::var(openai::ENV_KEY).unwrap_or_default(),
        "deepseek" => std::env::var(deepseek::ENV_KEY).unwrap_or_default(),
        "ollama" => std::env::var(ollama::ENV_KEY).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Return user-facing env var names for the backend.
#[must_use]
pub fn format_backend_env_keys(backend: &str) -> String {
    match backend {
        "gemini" => format!("{} or {}", gemini::ENV_KEY, gemini::ENV_KEY_FALLBACK),
        "kimi" => kimi::ENV_KEY.to_string(),
        "claude" => claude::ENV_KEY.to_string(),
        "openai" => openai::ENV_KEY.to_string(),
        "deepseek" => deepseek::ENV_KEY.to_string(),
        "ollama" => ollama::ENV_KEY.to_string(),
        _ => "AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY (or AWS_PROFILE)".to_string(),
    }
}

/// Detect which backend has a key configured.
///
/// Priority: gemini → kimi → claude → openai → deepseek → bedrock → ollama.
/// Returns `None` if no backend is configured.
///
/// Bedrock requires that AWS credentials actually appear to be configured —
/// not just `AWS_REGION` — otherwise every extraction chunk would fail with
/// "AWS credentials not configured" once we tried to call the API. The
/// check is via [`bedrock::credentials_appear_configured`], which inspects
/// the standard credential-provider env vars (access key + secret, profile,
/// web identity, container creds). The actual credential chain is still
/// resolved at call time by the AWS SDK.
#[must_use]
pub fn detect_backend() -> Option<String> {
    detect_backend_with(&crate::providers::load_custom_providers())
}

/// Detects the backend like [`detect_backend`], but against an explicit
/// custom-provider set.
///
/// Built-in backends take priority (gemini → kimi → claude → openai → deepseek →
/// bedrock → ollama); only then are custom providers consulted, in registry
/// order, returning the first whose `env_key` is set (#1084). Splitting the
/// custom set out makes the priority ordering testable without a `providers.json`.
#[must_use]
pub fn detect_backend_with(
    custom: &indexmap::IndexMap<String, crate::providers::CustomProvider>,
) -> Option<String> {
    for backend in ["gemini", "kimi", "claude", "openai", "deepseek"] {
        if !get_backend_api_key(backend).is_empty() {
            return Some(backend.to_string());
        }
    }
    if bedrock::credentials_appear_configured() {
        return Some("bedrock".to_string());
    }
    // Ollama: checked before custom providers to avoid shadowing a local model.
    if let Ok(url) = std::env::var(ollama::BASE_URL_ENV)
        && !url.is_empty()
    {
        ollama::validate_ollama_base_url(&url);
        return Some("ollama".to_string());
    }
    // Custom providers last: a configured `env_key` selects the provider.
    for (name, provider) in custom {
        if std::env::var(&provider.env_key).is_ok_and(|v| !v.is_empty()) {
            return Some(name.clone());
        }
    }
    None
}

/// Every environment variable [`detect_backend`] consults to auto-select a
/// built-in backend, in detection-priority order.
///
/// Built from the per-backend `ENV_KEY` constants, [`bedrock::CREDENTIAL_ENV_VARS`],
/// and [`ollama::BASE_URL_ENV`] — the same names the detection logic above reads —
/// so the list can never drift from it. Custom-provider `env_key`s are excluded
/// because they are registry-defined, not statically known. Tests scrub exactly
/// this set to drive the no-backend / custom-provider path deterministically.
#[must_use]
pub fn backend_selection_env_vars() -> Vec<&'static str> {
    let mut vars = vec![
        gemini::ENV_KEY,
        gemini::ENV_KEY_FALLBACK,
        kimi::ENV_KEY,
        claude::ENV_KEY,
        openai::ENV_KEY,
        deepseek::ENV_KEY,
    ];
    vars.extend_from_slice(bedrock::CREDENTIAL_ENV_VARS);
    vars.push(ollama::BASE_URL_ENV);
    vars
}
