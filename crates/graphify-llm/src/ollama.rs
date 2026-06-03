//! Ollama backend — local `OpenAI`-compatible server.
//!
//! Ports the ollama section in `graphify-py/graphify/llm.py`.

use crate::kimi::call_plain_openai_compat;
use crate::openai_compat::{
    OllamaOptions, OpenAiRequest, api_timeout, call_openai_compat, derive_ollama_num_ctx,
    resolve_max_tokens,
};
use crate::{LlmBackend, LlmError, LlmResponse};

/// Default model.
pub const DEFAULT_MODEL: &str = "qwen2.5-coder:7b";
/// API key env var (any non-empty value accepted; Ollama ignores it).
pub const ENV_KEY: &str = "OLLAMA_API_KEY";
/// Base URL env var.
pub const BASE_URL_ENV: &str = "OLLAMA_BASE_URL";
/// Model env var.
pub const MODEL_ENV: &str = "OLLAMA_MODEL";
const DEFAULT_BASE_URL: &str = "http://localhost:11434/v1";

/// Ollama backend.
pub struct OllamaBackend {
    api_key: String,
    base_url: String,
}

impl OllamaBackend {
    /// Create from environment.
    #[must_use]
    pub fn from_env() -> Self {
        let api_key = std::env::var(ENV_KEY).unwrap_or_default();
        let base_url = std::env::var(BASE_URL_ENV)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Self { api_key, base_url }
    }

    /// Create with explicit parameters.
    #[must_use]
    pub fn new(api_key: String, base_url: String) -> Self {
        Self { api_key, base_url }
    }
}

impl LlmBackend for OllamaBackend {
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "ollama"
    }

    /// Extracts the last user message for `num_ctx` estimation, then calls [`call_ollama`].
    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        // Extract user message text for num_ctx estimation.
        let user_msg = messages
            .iter()
            .filter_map(|m| {
                if m.get("role")?.as_str()? == "user" {
                    m.get("content")?.as_str()
                } else {
                    None
                }
            })
            .next_back()
            .unwrap_or("");

        call_ollama(
            &self.api_key,
            &self.base_url,
            model,
            messages,
            max_tokens,
            user_msg,
        )
    }

    /// Delegates to the shared tiktoken-based estimator.
    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Call Ollama via `OpenAI`-compat layer with `num_ctx` injection.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_ollama(
    api_key: &str,
    base_url: &str,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    user_message: &str,
) -> Result<LlmResponse, LlmError> {
    let num_ctx = resolve_num_ctx(user_message, max_tokens);
    let keep_alive = std::env::var("GRAPHIFY_OLLAMA_KEEP_ALIVE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "30m".to_string());

    let req = OpenAiRequest {
        base_url,
        api_key,
        model,
        messages: messages.to_vec(),
        temperature: Some(0.0),
        reasoning_effort: None,
        max_completion_tokens: max_tokens,
        disable_thinking: false,
        ollama_options: Some(OllamaOptions {
            num_ctx,
            keep_alive,
        }),
        backend_name: "ollama",
        timeout: api_timeout(),
    };
    call_openai_compat(&req)
}

/// Resolve `num_ctx` from env or auto-derive from message length.
#[must_use]
pub fn resolve_num_ctx(user_message: &str, max_completion_tokens: u32) -> u32 {
    let estimated_input = user_message.len() / crate::tokenizer::CHARS_PER_TOKEN + 400;
    let auto = derive_ollama_num_ctx(user_message, max_completion_tokens);

    if let Some(raw) = std::env::var("GRAPHIFY_OLLAMA_NUM_CTX")
        .ok()
        .filter(|s| !s.is_empty())
    {
        if let Ok(v) = raw.parse::<u32>() {
            // Warn if pinned value is smaller than estimated input.
            let estimated_u32 = u32::try_from(estimated_input).unwrap_or(u32::MAX);
            if v < estimated_u32 {
                eprintln!(
                    "[graphify] warning: GRAPHIFY_OLLAMA_NUM_CTX={v} is smaller than \
                     the estimated chunk input (~{estimated_input} tokens). Ollama will \
                     silently truncate the prompt and return empty responses. \
                     Try --token-budget {} or increase NUM_CTX.",
                    (estimated_u32 / 3).max(1024)
                );
            }
            v
        } else {
            eprintln!(
                "[graphify] GRAPHIFY_OLLAMA_NUM_CTX={raw:?} is not a valid integer; \
                 using auto-derived value ({auto})."
            );
            auto
        }
    } else {
        auto
    }
}

/// Plain-text call for the LLM tiebreaker path.
///
/// # Errors
/// Returns [`LlmError::Security`] if the URL fails SSRF validation, or
/// [`LlmError::Http`] / [`LlmError::Parse`] on transport errors.
pub fn call_ollama_plain(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
) -> Result<String, LlmError> {
    call_plain_openai_compat(&crate::kimi::PlainOpenAiRequest {
        base_url,
        api_key,
        model,
        prompt,
        temperature: Some(0.0),
        reasoning_effort: None,
        disable_thinking: false,
        max_tokens,
    })
}

/// Hosts that are always a metadata / SSRF target regardless of DNS resolution.
const METADATA_HOSTS: &[&str] = &[
    "metadata.google.internal",
    "metadata.google.com",
    "0.0.0.0",
    "::",
    "[::]",
];

/// Resolve `host` to its IP addresses via the system resolver (getaddrinfo
/// equivalent). Returns an empty vec on failure, matching Python's
/// fail-open-to-`False` behaviour on a resolution error.
///
/// This resolution happens at validation time; the HTTP client re-resolves at
/// send time, so a hostname is theoretically subject to TOCTOU / DNS-rebinding
/// (a name that resolves to a public IP here could map to `169.254.169.254` at
/// request time). That residual risk is accepted under graphify's threat model:
/// `OLLAMA_BASE_URL` is local user configuration, not attacker-supplied input,
/// and the check (F3) targets accidental/static metadata targets, not an active
/// network attacker who already controls the user's resolver.
fn resolve_host_ips(host: &str) -> Vec<std::net::IpAddr> {
    use std::net::ToSocketAddrs;
    // `ToSocketAddrs` requires a port; 0 is fine, we only read the resolved IP.
    (host, 0u16)
        .to_socket_addrs()
        .map(|iter| iter.map(|sa| sa.ip()).collect())
        .unwrap_or_default()
}

/// `true` if `host` is, or resolves to, a link-local / cloud-metadata address.
///
/// Uses the real system resolver. See [`ollama_host_is_link_local_or_metadata_with`].
#[must_use]
pub fn ollama_host_is_link_local_or_metadata(host: &str) -> bool {
    ollama_host_is_link_local_or_metadata_with(host, resolve_host_ips)
}

/// `true` if `host` is, or resolves to, a link-local / cloud-metadata address,
/// using the supplied `resolve` function for DNS (injectable for testing).
///
/// A name pointing at `169.254.169.254` is caught too, not just literal IPs.
/// General private/LAN addresses are deliberately NOT treated as metadata:
/// people do run Ollama on trusted LAN boxes, so those only warn.
#[must_use]
pub fn ollama_host_is_link_local_or_metadata_with(
    host: &str,
    resolve: impl Fn(&str) -> Vec<std::net::IpAddr>,
) -> bool {
    use std::net::IpAddr;
    if METADATA_HOSTS.contains(&host) {
        return true;
    }
    if host.starts_with("169.254.") {
        // link-local literal, includes the metadata IP 169.254.169.254
        return true;
    }
    resolve(host).into_iter().any(|ip| match ip {
        IpAddr::V4(v4) => v4.is_link_local(), // 169.254.0.0/16
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80, // fe80::/10
    })
}

/// Validate the Ollama base URL; warn on a risky-but-allowed target and
/// hard-block link-local / cloud-metadata addresses (F3).
///
/// Sending an entire corpus to a non-loopback `http://` endpoint silently leaks
/// proprietary code, but some users genuinely run Ollama on a LAN host they
/// trust, so a general non-loopback target only warns. A link-local or cloud
/// metadata address (169.254.x, `metadata.google.*`, or any host that resolves
/// to one) is never a legitimate Ollama host and is a classic SSRF target, so we
/// fail closed with [`LlmError::OllamaUrlBlocked`] there regardless of `warn`.
/// Pass `warn = false` for an early gate that should hard-block but leave the
/// user-facing warning to the later in-flow call.
///
/// # Errors
///
/// Returns [`LlmError::OllamaUrlBlocked`] when the host is, or resolves to, a
/// link-local / cloud-metadata address.
pub fn validate_ollama_base_url(url: &str, warn: bool) -> Result<(), LlmError> {
    let Ok(parsed) = url::Url::parse(url) else {
        if warn {
            eprintln!("[graphify] WARNING: OLLAMA_BASE_URL={url:?} is not a parseable URL.");
        }
        return Ok(());
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        if warn {
            eprintln!(
                "[graphify] WARNING: OLLAMA_BASE_URL has unexpected scheme {:?}; expected http or https.",
                parsed.scheme()
            );
        }
        return Ok(());
    }
    // `url::host_str()` returns an IPv6 literal WITH brackets (`[fe80::1]`);
    // strip them so the address parses as an IP for the checks below, matching
    // Python's `urlparse().hostname` (which yields the bracketless form).
    let raw_host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let host = raw_host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&raw_host)
        .to_string();
    if ollama_host_is_link_local_or_metadata(&host) {
        return Err(LlmError::OllamaUrlBlocked(host));
    }
    // Parse the host as an IP and use `IpAddr::is_loopback` (covers 127.0.0.0/8
    // and `::1`) rather than a `starts_with("127.")` prefix, so a hostname like
    // `127.evil.com` is correctly treated as non-loopback (and warned about).
    // Deliberate divergence from graphify-py's literal `startswith("127.")`.
    let is_loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if warn && !is_loopback {
        let scheme_note = if parsed.scheme() == "http" {
            " (UNENCRYPTED)"
        } else {
            ""
        };
        eprintln!(
            "[graphify] WARNING: OLLAMA_BASE_URL points to non-loopback host {host:?}{scheme_note}. \
             Your full corpus will be sent to that endpoint. \
             Set OLLAMA_BASE_URL=http://localhost:11434/v1 to keep extraction local."
        );
    }
    Ok(())
}

/// Default max tokens for ollama.
#[must_use]
pub fn default_max_tokens() -> u32 {
    resolve_max_tokens(16_384)
}
