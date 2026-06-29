//! Coverage tests for the pure helper functions in `openai_compat`.

#![allow(clippy::expect_used, clippy::float_cmp, unsafe_code)]

use graphify_llm::openai_compat::{
    OpenAiRequest, api_timeout, call_openai_compat, derive_ollama_num_ctx, extraction_messages,
    model_requires_default_temperature, plain_messages, resolve_max_retries, resolve_max_tokens,
    resolve_temperature, safe_parse_response,
};
use serde_json::json;
use std::time::Duration;

/// Scoped env-var helper.
struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self { saved: vec![] }
    }
    fn set(&mut self, k: &str, v: &str) -> &mut Self {
        let prev = std::env::var(k).ok();
        unsafe { std::env::set_var(k, v) };
        self.saved.push((k.to_string(), prev));
        self
    }
    fn remove(&mut self, k: &str) -> &mut Self {
        let prev = std::env::var(k).ok();
        unsafe { std::env::remove_var(k) };
        self.saved.push((k.to_string(), prev));
        self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, prev) in self.saved.drain(..).rev() {
            match prev {
                Some(v) => unsafe { std::env::set_var(&k, &v) },
                None => unsafe { std::env::remove_var(&k) },
            }
        }
    }
}

// ── api_timeout ─────────────────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn api_timeout_default_is_10_minutes() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_API_TIMEOUT");
    assert_eq!(api_timeout(), Duration::from_mins(10));
}

#[test]
#[serial_test::serial(env)]
fn api_timeout_honours_env_var() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_API_TIMEOUT", "30");
    assert_eq!(api_timeout(), Duration::from_secs(30));
}

#[test]
#[serial_test::serial(env)]
fn api_timeout_accepts_float() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_API_TIMEOUT", "15.5");
    assert_eq!(api_timeout(), Duration::from_secs_f64(15.5));
}

#[test]
#[serial_test::serial(env)]
fn api_timeout_ignores_invalid_value() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_API_TIMEOUT", "not-a-number");
    assert_eq!(api_timeout(), Duration::from_mins(10));
}

#[test]
#[serial_test::serial(env)]
fn api_timeout_ignores_zero_or_negative() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_API_TIMEOUT", "0");
    assert_eq!(api_timeout(), Duration::from_mins(10));
    g.set("GRAPHIFY_API_TIMEOUT", "-1");
    assert_eq!(api_timeout(), Duration::from_mins(10));
}

// ── resolve_max_tokens ──────────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn resolve_max_tokens_returns_default_without_env() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_MAX_OUTPUT_TOKENS");
    assert_eq!(resolve_max_tokens(8192), 8192);
}

#[test]
#[serial_test::serial(env)]
fn resolve_max_tokens_honours_env_var() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_MAX_OUTPUT_TOKENS", "4096");
    assert_eq!(resolve_max_tokens(8192), 4096);
}

#[test]
#[serial_test::serial(env)]
fn resolve_max_tokens_ignores_invalid() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_MAX_OUTPUT_TOKENS", "not-a-num");
    assert_eq!(resolve_max_tokens(2048), 2048);
}

#[test]
#[serial_test::serial(env)]
fn resolve_max_tokens_ignores_zero() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_MAX_OUTPUT_TOKENS", "0");
    assert_eq!(resolve_max_tokens(1024), 1024);
}

// ── derive_ollama_num_ctx ──────────────────────────────────────────────────

#[test]
fn derive_ollama_num_ctx_clamps_lower() {
    // Empty input → clamp to 8192.
    assert_eq!(derive_ollama_num_ctx("", 1000), 8192);
}

#[test]
fn derive_ollama_num_ctx_clamps_upper() {
    let huge = "x".repeat(2_000_000);
    assert_eq!(derive_ollama_num_ctx(&huge, 100_000), 131_072);
}

#[test]
fn derive_ollama_num_ctx_scales_with_input() {
    let small = derive_ollama_num_ctx("x".repeat(1000).as_str(), 4096);
    let large = derive_ollama_num_ctx("x".repeat(50_000).as_str(), 4096);
    assert!(large > small || (small == 8192 && large == 8192));
}

// ── extraction_messages / plain_messages ───────────────────────────────────

#[test]
fn extraction_messages_includes_system_and_user() {
    let msgs = extraction_messages("hello");
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "hello");
}

#[test]
fn plain_messages_user_only() {
    let msgs = plain_messages("hi");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], "hi");
}

// ── safe_parse_response ────────────────────────────────────────────────────

#[test]
fn safe_parse_response_parses_normal_json() {
    let v = safe_parse_response(r#"{"nodes":[{"id":"x"}],"edges":[]}"#);
    assert_eq!(v["nodes"][0]["id"], "x");
}

#[test]
fn safe_parse_response_returns_empty_on_oversized() {
    // LLM_JSON_MAX_BYTES is 4 MiB; build something bigger.
    let mut big = String::with_capacity(5 * 1024 * 1024);
    big.push('{');
    big.push_str(&"\"x\":".repeat(1_000_000));
    big.push_str("\"end\"}");
    let v = safe_parse_response(&big);
    assert!(v["nodes"].as_array().expect("array field").is_empty());
    assert!(v["edges"].as_array().expect("array field").is_empty());
    assert!(v["hyperedges"].as_array().expect("array field").is_empty());
}

#[test]
fn safe_parse_response_handles_markdown_fences() {
    let v = safe_parse_response("```json\n{\"nodes\":[],\"edges\":[]}\n```");
    assert!(v["nodes"].as_array().expect("array field").is_empty());
}

// ── call_openai_compat with bad URL hits SSRF guard ────────────────────────

#[test]
fn call_openai_compat_rejects_private_ip() {
    let req = OpenAiRequest {
        base_url: "http://127.0.0.1:1",
        api_key: "fake",
        model: "fake-model",
        messages: vec![json!({"role": "user", "content": "hi"})],
        temperature: Some(0.0),
        reasoning_effort: None,
        max_completion_tokens: 16,
        disable_thinking: false,
        custom_extra_body: None,
        ollama_options: None,
        backend_name: "openai",
        timeout: Duration::from_secs(1),
    };
    let result = call_openai_compat(&req);
    assert!(result.is_err());
}

#[test]
fn call_openai_compat_rejects_bad_scheme() {
    let req = OpenAiRequest {
        base_url: "ftp://example.com",
        api_key: "fake",
        model: "fake-model",
        messages: vec![],
        temperature: None,
        reasoning_effort: None,
        max_completion_tokens: 16,
        disable_thinking: false,
        custom_extra_body: None,
        ollama_options: None,
        backend_name: "openai",
        timeout: Duration::from_secs(1),
    };
    assert!(call_openai_compat(&req).is_err());
}

// ── temperature resolution (#1191) ───────────────────────────────────────────

#[test]
fn model_requires_default_temperature_true_for_reasoning_models() {
    for model in [
        "o1",
        "o1-preview",
        "o1-mini",
        "o3",
        "o3-mini",
        "o4-mini",
        "gpt-5",
        "gpt-5-mini",
        "openai/o3-mini",
    ] {
        assert!(
            model_requires_default_temperature(model),
            "{model} should require default temperature"
        );
    }
}

#[test]
fn model_requires_default_temperature_false_for_normal_models() {
    for model in [
        "gpt-4.1-mini",
        "gpt-4o",
        "gpt-4.1",
        "kimi-k2.6",
        "deepseek-v4-flash",
        "",
        "o1x",
        "go3",
    ] {
        assert!(
            !model_requires_default_temperature(model),
            "{model} should NOT require default temperature"
        );
    }
}

#[serial_test::serial(env)]
#[test]
fn resolve_temperature_default_for_normal_model() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_LLM_TEMPERATURE");
    assert_eq!(resolve_temperature(Some(0.0), "gpt-4.1-mini"), Some(0.0));
}

#[serial_test::serial(env)]
#[test]
fn resolve_temperature_omitted_for_reasoning_model() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_LLM_TEMPERATURE");
    assert_eq!(resolve_temperature(Some(0.0), "o3-mini"), None);
    assert_eq!(resolve_temperature(Some(0.0), "gpt-5"), None);
}

#[serial_test::serial(env)]
#[test]
fn resolve_temperature_env_var_numeric_overrides() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_LLM_TEMPERATURE", "0.7");
    assert_eq!(resolve_temperature(Some(0.0), "gpt-4.1-mini"), Some(0.7));
    // Env var wins even for a reasoning model (explicit user choice).
    assert_eq!(resolve_temperature(Some(0.0), "o3-mini"), Some(0.7));
}

#[serial_test::serial(env)]
#[test]
fn resolve_temperature_env_var_none_omits() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_LLM_TEMPERATURE", "none");
    assert_eq!(resolve_temperature(Some(0.0), "gpt-4.1-mini"), None);
}

#[serial_test::serial(env)]
#[test]
fn resolve_temperature_env_var_invalid_falls_back() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_LLM_TEMPERATURE", "hot");
    assert_eq!(resolve_temperature(Some(0.0), "gpt-4.1-mini"), Some(0.0));
    assert_eq!(resolve_temperature(Some(0.0), "o3-mini"), None);
}

// ── resolve_max_retries ──────────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn resolve_max_retries_default_and_env() {
    // Default retry count is generous (so 429s are absorbed, #1523); env overrides.
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_MAX_RETRIES");
    assert!(resolve_max_retries() >= 5, "default should be generous");
    g.set("GRAPHIFY_MAX_RETRIES", "10");
    assert_eq!(resolve_max_retries(), 10);
    g.set("GRAPHIFY_MAX_RETRIES", "0");
    assert_eq!(resolve_max_retries(), 0, "disable is allowed");
    g.set("GRAPHIFY_MAX_RETRIES", "bogus");
    assert!(resolve_max_retries() >= 5, "invalid -> default");
}
