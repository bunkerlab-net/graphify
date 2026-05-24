//! Error-path tests for `call_llm`, `extract_files_direct`, retry helpers, and
//! parse helpers — covers code paths reachable without making real HTTP calls.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    unsafe_code
)]

use graphify_llm::{LlmError, LlmResponse, call_llm, empty_fragment, extract_files_direct};
use serde_json::json;

/// Scoped env-var helper.
struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self { saved: vec![] }
    }
    fn remove(&mut self, key: &str) -> &mut Self {
        let prev = std::env::var(key).ok();
        // SAFETY: test-only env mutation.
        unsafe { std::env::remove_var(key) };
        self.saved.push((key.to_string(), prev));
        self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.saved.drain(..).rev() {
            match prev {
                Some(v) => unsafe { std::env::set_var(&key, &v) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }
    }
}

fn clear_backend_envs(g: &mut EnvGuard) {
    for key in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "DEEPSEEK_API_KEY",
        "MOONSHOT_API_KEY",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "OLLAMA_API_KEY",
    ] {
        g.remove(key);
    }
}

// ── call_llm ────────────────────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn call_llm_unknown_backend_errors() {
    let result = call_llm("hello", "no_such_backend", 16);
    assert!(matches!(result, Err(LlmError::UnknownBackend(_, _))));
}

#[test]
#[serial_test::serial(env)]
fn call_llm_missing_api_key_errors() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    // OpenAI requires an API key.
    let result = call_llm("hello", "openai", 16);
    assert!(matches!(result, Err(LlmError::NoApiKey(_))));
}

#[test]
#[serial_test::serial(env)]
fn call_llm_gemini_missing_key_errors() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    let result = call_llm("hello", "gemini", 16);
    assert!(matches!(result, Err(LlmError::NoApiKey(_))));
}

#[test]
#[serial_test::serial(env)]
fn call_llm_kimi_missing_key_errors() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    let result = call_llm("hello", "kimi", 16);
    assert!(matches!(result, Err(LlmError::NoApiKey(_))));
}

#[test]
#[serial_test::serial(env)]
fn call_llm_deepseek_missing_key_errors() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    let result = call_llm("hello", "deepseek", 16);
    assert!(matches!(result, Err(LlmError::NoApiKey(_))));
}

// ── extract_files_direct ───────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn extract_files_direct_unknown_backend() {
    let result = extract_files_direct(
        &[],
        "no_such_backend",
        None,
        None,
        std::path::Path::new("."),
    );
    assert!(matches!(result, Err(LlmError::UnknownBackend(_, _))));
}

#[test]
#[serial_test::serial(env)]
fn extract_files_direct_missing_api_key() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    let result = extract_files_direct(&[], "openai", None, None, std::path::Path::new("."));
    assert!(matches!(result, Err(LlmError::NoApiKey(_))));
}

#[test]
#[serial_test::serial(env)]
fn extract_files_direct_with_empty_string_api_key_still_errors() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    // Explicit empty key still falls through to env lookup (also empty).
    let result = extract_files_direct(&[], "openai", Some(""), None, std::path::Path::new("."));
    assert!(matches!(result, Err(LlmError::NoApiKey(_))));
}

// ── parse helpers ──────────────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn empty_fragment_has_expected_shape() {
    let v = empty_fragment();
    assert!(v["nodes"].as_array().unwrap().is_empty());
    assert!(v["edges"].as_array().unwrap().is_empty());
    assert!(v["hyperedges"].as_array().unwrap().is_empty());
}

// ── LlmResponse merging via retry helpers (only public surface) ───────────

#[test]
#[serial_test::serial(env)]
fn llm_response_default_is_sensible() {
    let r = LlmResponse {
        nodes: vec![],
        edges: vec![],
        hyperedges: vec![],
        input_tokens: 0,
        output_tokens: 0,
        model: "model".into(),
        finish_reason: "stop".into(),
        elapsed_seconds: 0.0,
        failed_chunk_indices: vec![],
    };
    assert_eq!(r.input_tokens, 0);
}

// ── looks_like_context_exceeded ────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn looks_like_context_exceeded_detects_common_markers() {
    use graphify_llm::looks_like_context_exceeded;

    for msg in [
        "context size exceeded",
        "context length too long",
        "context_length_exceeded",
        "your prompt is too long",
        "exceeds the available context",
        "maximum context window exceeded",
        "request has too many tokens",
        "n_keep > n_ctx",
    ] {
        let err = LlmError::Http(msg.to_string());
        assert!(
            looks_like_context_exceeded(&err),
            "expected marker '{msg}' to be detected"
        );
    }
}

#[test]
#[serial_test::serial(env)]
fn looks_like_context_exceeded_ignores_other_errors() {
    use graphify_llm::looks_like_context_exceeded;
    let err = LlmError::Http("connection refused".to_string());
    assert!(!looks_like_context_exceeded(&err));
}

#[test]
#[serial_test::serial(env)]
fn looks_like_context_exceeded_dyn_works() {
    use graphify_llm::looks_like_context_exceeded_dyn;
    #[derive(Debug)]
    struct E(&'static str);
    impl std::fmt::Display for E {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.fmt(f)
        }
    }
    impl std::error::Error for E {}

    let e: Box<dyn std::error::Error + Send + Sync> =
        Box::new(E("the context_length_exceeded today"));
    assert!(looks_like_context_exceeded_dyn(e.as_ref()));
    let e2: Box<dyn std::error::Error + Send + Sync> = Box::new(E("dns failure"));
    assert!(!looks_like_context_exceeded_dyn(e2.as_ref()));
}

// ── empty_fragment used in fallback paths ──────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn empty_fragment_round_trips_serialization() {
    let v = empty_fragment();
    let text = serde_json::to_string(&v).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, json!({"nodes": [], "edges": [], "hyperedges": []}));
}
