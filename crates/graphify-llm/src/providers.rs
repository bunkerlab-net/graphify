//! Custom LLM provider registry loaded from `providers.json` (#1084).
//!
//! Mirrors `_custom_providers_path` / `_load_custom_providers` from
//! `graphify-py/graphify/llm.py`. A custom provider is an OpenAI-compatible
//! endpoint declared by the user (`graphify provider add …`) so backends beyond
//! the built-in set can drive extraction and community labelling.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::Value;

use crate::backends::{BACKENDS, Pricing};

/// A user-declared OpenAI-compatible provider.
#[derive(Debug, Clone)]
pub struct CustomProvider {
    /// Provider name (the `--backend` selector).
    pub name: String,
    /// OpenAI-compatible base URL (e.g. `https://integrate.api.nvidia.com/v1`).
    pub base_url: String,
    /// Default model string when the caller does not override it.
    pub default_model: String,
    /// Environment variable holding the API key.
    pub env_key: String,
    /// Pricing for cost estimation (defaults to zero when omitted).
    pub pricing: Pricing,
    /// Sampling temperature (defaults to 0).
    pub temperature: f64,
    /// Default output-token budget for extraction, before the
    /// `GRAPHIFY_MAX_OUTPUT_TOKENS` override. Mirrors Python's
    /// `cfg.get("max_completion_tokens", 8192)` on the OpenAI-compatible path.
    pub max_completion_tokens: u32,
}

/// Fallback output-token budget when a provider omits `max_completion_tokens`,
/// matching Python's `cfg.get("max_completion_tokens", 8192)`.
const DEFAULT_MAX_COMPLETION_TOKENS: u32 = 8192;

/// Parse a `max_completion_tokens` JSON value, accepting either an integer or a
/// finite non-negative float, truncated toward zero (e.g. `8192.0` and `8192.9`
/// both yield `8192`). Returns `None` for missing, negative, non-finite, or
/// out-of-`u32`-range values so the caller falls back to the default rather than
/// silently dropping a hand-written float budget.
fn max_completion_tokens_from(v: &Value) -> Option<u32> {
    if let Some(n) = v.as_u64() {
        return u32::try_from(n).ok();
    }
    let f = v.as_f64()?;
    if !f.is_finite() || f < 0.0 {
        return None;
    }
    // Truncation is intentional: a token budget is an integer count.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = f as u64;
    u32::try_from(n).ok()
}

/// Path to the `providers.json` registry.
///
/// `global == true` → `~/.graphify/providers.json`; otherwise the
/// project-local `.graphify/providers.json`. Mirrors Python
/// `_custom_providers_path`.
#[must_use]
pub fn custom_providers_path(global: bool) -> PathBuf {
    // When `$HOME` is unset the global path falls back to the local one rather
    // than resolving to a stray relative `.graphify/...` that would collide with
    // — and be read twice as — the local path.
    if global && let Some(home) = home_dir() {
        return home.join(".graphify").join("providers.json");
    }
    PathBuf::from(".graphify").join("providers.json")
}

/// User home directory from `$HOME` (matches the detect crate's resolution).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Load custom providers from the standard local + global `providers.json`
/// paths. Built-in names are never shadowed; pricing defaults to zero.
#[must_use]
pub fn load_custom_providers() -> IndexMap<String, CustomProvider> {
    load_custom_providers_from(&custom_providers_path(false), &custom_providers_path(true))
}

/// Load custom providers from explicit `local`/`global` registry files (in that
/// order — a later file overrides an earlier one for the same name, so `global`
/// wins, matching Python `_load_custom_providers`). Malformed files are skipped
/// silently, mirroring Python's broad `except`. Identical `local`/`global` paths
/// (e.g. when `$HOME` is unset) are read only once.
#[must_use]
pub fn load_custom_providers_from(local: &Path, global: &Path) -> IndexMap<String, CustomProvider> {
    let mut providers: IndexMap<String, CustomProvider> = IndexMap::new();
    let mut paths: Vec<&Path> = vec![local];
    if global != local {
        paths.push(global);
    }
    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        for (name, cfg) in map {
            if is_builtin_backend(&name) {
                continue;
            }
            let Some(obj) = cfg.as_object() else {
                continue;
            };
            // A provider needs all three required string fields to be usable;
            // `graphify provider add` already rejects a record missing any of
            // them, so a hand-edited registry entry that omits one (or leaves it
            // blank) is non-functional. Skip it rather than insert a half-formed
            // provider that can never authenticate or address an endpoint.
            let required = ["base_url", "default_model", "env_key"].map(|key| {
                obj.get(key)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            });
            let [Some(base_url), Some(default_model), Some(env_key)] = required else {
                continue;
            };
            let pricing = obj.get("pricing").and_then(Value::as_object).map_or(
                Pricing {
                    input: 0.0,
                    output: 0.0,
                },
                |p| Pricing {
                    input: p.get("input").and_then(Value::as_f64).unwrap_or(0.0),
                    output: p.get("output").and_then(Value::as_f64).unwrap_or(0.0),
                },
            );
            providers.insert(
                name.clone(),
                CustomProvider {
                    name,
                    base_url: base_url.to_string(),
                    default_model: default_model.to_string(),
                    env_key: env_key.to_string(),
                    pricing,
                    temperature: obj
                        .get("temperature")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    max_completion_tokens: obj
                        .get("max_completion_tokens")
                        .and_then(max_completion_tokens_from)
                        .unwrap_or(DEFAULT_MAX_COMPLETION_TOKENS),
                },
            );
        }
    }
    providers
}

/// Returns `true` if `name` is a built-in backend (which a custom provider may
/// never shadow).
#[must_use]
pub fn is_builtin_backend(name: &str) -> bool {
    BACKENDS.iter().any(|b| b.name == name)
}
