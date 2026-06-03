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

/// Environment opt-in that allows loading a project-local `providers.json` (F1).
const ALLOW_LOCAL_ENV: &str = "GRAPHIFY_ALLOW_LOCAL_PROVIDERS";

/// Structural safety check for a custom-provider `base_url` (F1).
///
/// A custom provider receives the full corpus plus the user's API key, so its
/// `base_url` is an exfiltration channel. We deliberately do NOT run the ingest
/// SSRF guard here: that blocks private/internal IPs, which would wrongly reject
/// legitimate on-prem corporate LLM gateways. Instead we reject non-`http(s)`
/// schemes outright and warn loudly when the corpus would leave over plaintext
/// `http` to a non-loopback host. The primary control against trusting injected
/// config is the [`ALLOW_LOCAL_ENV`] gate on project-local files.
///
/// Pass `warn = false` to run the structural check silently.
#[must_use]
pub fn provider_base_url_ok(base_url: &str, name: &str, warn: bool) -> bool {
    let Ok(parsed) = url::Url::parse(base_url) else {
        if warn {
            eprintln!(
                "[graphify] WARNING: provider {name:?} has an unparseable base_url; ignoring."
            );
        }
        return false;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        if warn {
            eprintln!(
                "[graphify] WARNING: provider {name:?} base_url scheme {:?} is not http/https; ignoring.",
                parsed.scheme()
            );
        }
        return false;
    }
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    // Parse the 127.0.0.0/8 case as an IP rather than a `starts_with("127.")`
    // prefix, so a hostname like `127.evil.com` is correctly treated as
    // non-loopback (and therefore warned about over plaintext http). This is a
    // deliberate divergence from graphify-py's literal `startswith("127.")`.
    let is_loopback = host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback());
    if warn && parsed.scheme() == "http" && !is_loopback {
        eprintln!(
            "[graphify] WARNING: provider {name:?} sends your corpus to {host:?} over plaintext \
             http. Use https unless this is a trusted local endpoint."
        );
    }
    true
}

/// `true` when the [`ALLOW_LOCAL_ENV`] opt-in is set to `1`/`true`/`yes`.
fn local_providers_allowed() -> bool {
    std::env::var(ALLOW_LOCAL_ENV)
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
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

/// Load custom providers from explicit `local`/`global` registry files.
///
/// A project-local `./.graphify/providers.json` travels with a cloned or shared
/// repo and defines where the corpus + API key are sent, so loading it silently
/// is a corpus/key exfiltration vector (F1). It is therefore **ignored by
/// default** — only the user's own `global` (`~/.graphify/providers.json`) is
/// trusted — and read solely when the [`ALLOW_LOCAL_ENV`] opt-in is set, in which
/// case it takes precedence (read first; **first occurrence of a name wins**).
/// A distinct project-local file that exists but is not opted in produces a
/// stderr warning. Built-in names are un-shadowable, each provider's `base_url`
/// must pass [`provider_base_url_ok`], malformed files are skipped silently, and
/// identical `local`/`global` paths (e.g. when `$HOME` is unset) are read once
/// without any local-gating warning.
#[must_use]
pub fn load_custom_providers_from(local: &Path, global: &Path) -> IndexMap<String, CustomProvider> {
    let mut providers: IndexMap<String, CustomProvider> = IndexMap::new();
    let allow_local = local_providers_allowed();
    let local_distinct = local != global;

    if local_distinct && !allow_local && local.is_file() {
        eprintln!(
            "[graphify] WARNING: ignoring project-local {} (custom providers control where your \
             corpus and API key are sent). Set {ALLOW_LOCAL_ENV}=1 to load it.",
            local.display()
        );
    }

    // Opted-in local is read first so it takes precedence on a name clash; the
    // opt-in itself is the trust gate. Otherwise only the trusted global is read.
    let mut paths: Vec<&Path> = Vec::new();
    if local_distinct && allow_local {
        paths.push(local);
    }
    paths.push(global);

    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        for (name, cfg) in map {
            // First occurrence of a name wins (built-ins are never shadowable).
            if is_builtin_backend(&name) || providers.contains_key(&name) {
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
            // Reject a non-http(s) base_url (and warn on plaintext egress): the
            // base_url decides where the corpus + API key are sent (F1).
            if !provider_base_url_ok(base_url, &name, true) {
                continue;
            }
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
