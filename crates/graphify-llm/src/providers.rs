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
}

/// Path to the `providers.json` registry.
///
/// `global == true` → `~/.graphify/providers.json`; otherwise the
/// project-local `.graphify/providers.json`. Mirrors Python
/// `_custom_providers_path`.
#[must_use]
pub fn custom_providers_path(global: bool) -> PathBuf {
    if global {
        home_dir()
            .unwrap_or_default()
            .join(".graphify")
            .join("providers.json")
    } else {
        PathBuf::from(".graphify").join("providers.json")
    }
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
/// order — a later file overrides an earlier one for the same name). Malformed
/// files are skipped silently, mirroring Python's broad `except`.
#[must_use]
pub fn load_custom_providers_from(local: &Path, global: &Path) -> IndexMap<String, CustomProvider> {
    let mut providers: IndexMap<String, CustomProvider> = IndexMap::new();
    for path in [local, global] {
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
                    base_url: obj
                        .get("base_url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    default_model: obj
                        .get("default_model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    env_key: obj
                        .get("env_key")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    pricing,
                    temperature: obj
                        .get("temperature")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
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
