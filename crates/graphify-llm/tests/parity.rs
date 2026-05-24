//! Parity tests for `graphify-llm`.
//!
//! Ports `graphify-py/graphify/tests/test_llm_backends.py`,
//! `test_ollama.py`, and `test_claude_cli_backend.py`.
//!
//! # Env-var isolation
//!
//! Tests that read env vars use a scoped `EnvGuard` RAII helper that restores
//! original values on drop. `std::env::set_var` / `remove_var` are `unsafe`
//! in Rust 2024; the `#![allow(unsafe_code)]` below permits their use in this
//! test-only file (matching the convention in other `graphify-*` parity tests).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    unsafe_code
)]

use graphify_llm::claude_cli::ClaudeRunner;
use graphify_llm::ollama::resolve_num_ctx;
use graphify_llm::{
    BACKENDS, LlmError, backend_config, detect_backend, estimate_cost, looks_like_context_exceeded,
    parse_llm_json, response_is_hollow,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Temporarily clear and optionally set env vars, run a closure, then restore.
struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self { saved: vec![] }
    }

    fn remove(&mut self, key: &str) -> &mut Self {
        let prev = std::env::var(key).ok();
        // SAFETY: test-only, single-threaded fixture pattern.
        unsafe { std::env::remove_var(key) };
        self.saved.push((key.to_string(), prev));
        self
    }

    fn set(&mut self, key: &str, value: &str) -> &mut Self {
        let prev = std::env::var(key).ok();
        // SAFETY: test-only, single-threaded fixture pattern.
        unsafe { std::env::set_var(key, value) };
        self.saved.push((key.to_string(), prev));
        self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.saved.drain(..).rev() {
            match prev {
                // SAFETY: test-only, restoring original state.
                Some(v) => unsafe { std::env::set_var(&key, &v) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }
    }
}

/// Clear all backend-related env vars.
fn clear_backend_envs(g: &mut EnvGuard) {
    for key in &[
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "MOONSHOT_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "OLLAMA_BASE_URL",
        "AWS_PROFILE",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_WEB_IDENTITY_TOKEN_FILE",
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    ] {
        g.remove(key);
    }
}

// ---------------------------------------------------------------------------
// test_llm_backends.py — backend detection & API key lookup
// ---------------------------------------------------------------------------

#[test]
#[serial_test::serial(env)]
fn test_gemini_accepts_gemini_api_key() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("GEMINI_API_KEY", "gemini-key");

    assert_eq!(detect_backend().as_deref(), Some("gemini"));
    assert_eq!(graphify_llm::get_backend_api_key("gemini"), "gemini-key");
}

#[test]
#[serial_test::serial(env)]
fn test_gemini_accepts_google_api_key() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("GOOGLE_API_KEY", "google-key");

    assert_eq!(detect_backend().as_deref(), Some("gemini"));
    assert_eq!(graphify_llm::get_backend_api_key("gemini"), "google-key");
}

#[test]
#[serial_test::serial(env)]
fn test_backend_detection_prefers_gemini() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("OPENAI_API_KEY", "openai-key");
    g.set("ANTHROPIC_API_KEY", "anthropic-key");
    g.set("MOONSHOT_API_KEY", "moonshot-key");
    g.set("GEMINI_API_KEY", "gemini-key");

    assert_eq!(detect_backend().as_deref(), Some("gemini"));
}

#[test]
#[serial_test::serial(env)]
fn test_openai_backend_detected() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("OPENAI_API_KEY", "openai-key");

    assert_eq!(detect_backend().as_deref(), Some("openai"));
    assert_eq!(graphify_llm::get_backend_api_key("openai"), "openai-key");
}

// ---------------------------------------------------------------------------
// test_llm_backends.py — context-exceeded detection
// ---------------------------------------------------------------------------

#[test]
fn test_looks_like_context_exceeded_matches_common_messages() {
    let msgs = [
        "Error code: 400 - {'error': 'Context size has been exceeded.'}",
        "n_keep: 22374 >= n_ctx: 4096",
        "context_length_exceeded: This model's maximum context length is 8192 tokens",
        "exceeds the available context size",
        "The prompt is too long for this model.",
    ];
    for m in &msgs {
        let err = LlmError::Http(m.to_string());
        assert!(
            looks_like_context_exceeded(&err),
            "expected context-exceeded for: {m}"
        );
    }
}

#[test]
fn test_looks_like_context_exceeded_ignores_unrelated_errors() {
    let msgs = [
        "timeout",
        "rate limit",
        "401 unauthorized",
        "connection refused",
    ];
    for m in &msgs {
        let err = LlmError::Http(m.to_string());
        assert!(
            !looks_like_context_exceeded(&err),
            "expected no context-exceeded for: {m}"
        );
    }
}

// ---------------------------------------------------------------------------
// test_llm_backends.py — hollow-response detection
// ---------------------------------------------------------------------------

#[test]
fn test_response_is_hollow_flags_empty_string() {
    let parsed = json!({"nodes": [], "edges": [], "hyperedges": []});
    assert!(response_is_hollow(Some(""), &parsed));
}

#[test]
fn test_response_is_hollow_flags_none_content() {
    let parsed = json!({"nodes": [], "edges": [], "hyperedges": []});
    assert!(response_is_hollow(None, &parsed));
}

#[test]
fn test_response_is_hollow_flags_whitespace_only() {
    let parsed = json!({"nodes": [], "edges": [], "hyperedges": []});
    assert!(response_is_hollow(Some("   \n\t  "), &parsed));
}

#[test]
fn test_response_is_hollow_flags_parsed_but_no_nodes_or_edges() {
    let parsed_empty_obj: Value = json!({});
    assert!(response_is_hollow(
        Some(r#"{"sorry": "I cannot"}"#),
        &parsed_empty_obj
    ));
    let parsed_empty_arrs = json!({"nodes": [], "edges": [], "hyperedges": []});
    assert!(response_is_hollow(Some("{}"), &parsed_empty_arrs));
}

#[test]
fn test_response_is_hollow_accepts_real_extraction_nodes() {
    let parsed = json!({"nodes": [{"id": "x"}], "edges": [], "hyperedges": []});
    assert!(!response_is_hollow(
        Some(r#"{"nodes":[{"id":"x"}]}"#),
        &parsed
    ));
}

#[test]
fn test_response_is_hollow_accepts_real_extraction_edges() {
    let parsed = json!({"nodes": [], "edges": [{"source": "a", "target": "b"}], "hyperedges": []});
    assert!(!response_is_hollow(Some(r#"{"edges":[...]}"#), &parsed));
}

// ---------------------------------------------------------------------------
// test_llm_backends.py — parse_llm_json
// ---------------------------------------------------------------------------

#[test]
fn test_parse_llm_json_strips_markdown_fence() {
    let raw = "```json\n{\"nodes\":[]}\n```";
    let parsed = parse_llm_json(raw);
    assert_eq!(parsed["nodes"], json!([]));
}

#[test]
fn test_parse_llm_json_handles_plain_json() {
    let raw = r#"{"nodes":[{"id":"x"}],"edges":[],"hyperedges":[]}"#;
    let parsed = parse_llm_json(raw);
    assert_eq!(parsed["nodes"][0]["id"], "x");
}

#[test]
fn test_parse_llm_json_returns_empty_fragment_on_invalid() {
    let parsed = parse_llm_json("not json at all");
    assert_eq!(parsed["nodes"], json!([]));
    assert_eq!(parsed["edges"], json!([]));
}

#[test]
fn test_parse_llm_json_returns_empty_fragment_on_truncated() {
    let parsed = parse_llm_json(r#"{"nodes": [{"id":"#);
    assert_eq!(parsed["nodes"], json!([]));
}

// ---------------------------------------------------------------------------
// test_llm_backends.py — estimate_cost
// ---------------------------------------------------------------------------

#[test]
fn test_estimate_cost_claude_cli_is_zero() {
    let cost = estimate_cost("claude-cli", 1_000_000, 1_000_000);
    assert_eq!(cost, 0.0_f64, "claude-cli must have zero cost");
}

#[test]
fn test_estimate_cost_nonzero_for_paid_backend() {
    // Claude: $3 input / $15 output per 1M tokens.
    let cost = estimate_cost("claude", 1_000_000, 0);
    assert!(
        (cost - 3.0_f64).abs() < 1e-9,
        "expected $3 for 1M input tokens on claude"
    );
}

#[test]
fn test_estimate_cost_unknown_backend_returns_zero() {
    let cost = estimate_cost("nonexistent-backend", 1_000_000, 1_000_000);
    assert_eq!(cost, 0.0_f64);
}

// ---------------------------------------------------------------------------
// test_ollama.py — backend registry
// ---------------------------------------------------------------------------

#[test]
fn test_ollama_in_backends() {
    let cfg = backend_config("ollama").expect("ollama must be registered");
    assert_eq!(cfg.pricing.input, 0.0);
    assert_eq!(cfg.pricing.output, 0.0);
    assert!(cfg.default_max_tokens > 0);
}

#[test]
fn test_claude_cli_in_backends() {
    let cfg = backend_config("claude-cli").expect("claude-cli must be registered");
    assert_eq!(cfg.pricing.input, 0.0);
    assert_eq!(cfg.pricing.output, 0.0);
}

#[test]
fn test_all_expected_backends_registered() {
    let names: Vec<&str> = BACKENDS.iter().map(|b| b.name).collect();
    for expected in &[
        "gemini",
        "kimi",
        "claude",
        "openai",
        "deepseek",
        "ollama",
        "bedrock",
        "claude-cli",
    ] {
        assert!(names.contains(expected), "missing backend: {expected}");
    }
}

// ---------------------------------------------------------------------------
// test_ollama.py — backend detection with OLLAMA_BASE_URL
// ---------------------------------------------------------------------------

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_ollama_from_base_url() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("OLLAMA_BASE_URL", "http://localhost:11434/v1");

    assert_eq!(detect_backend().as_deref(), Some("ollama"));
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_kimi_beats_ollama() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("MOONSHOT_API_KEY", "test-key");
    g.set("OLLAMA_BASE_URL", "http://localhost:11434/v1");

    assert_eq!(detect_backend().as_deref(), Some("kimi"));
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_claude_beats_ollama() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("ANTHROPIC_API_KEY", "sk-test");
    g.set("OLLAMA_BASE_URL", "http://localhost:11434/v1");

    assert_eq!(detect_backend().as_deref(), Some("claude"));
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_none_without_envvars() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);

    assert_eq!(detect_backend(), None);
}

// ---------------------------------------------------------------------------
// Bedrock auto-detection — only triggers when credentials look configured.
// The pre-port behaviour (auto-select Bedrock when *only* AWS_REGION was set)
// led to every extraction chunk failing with "AWS credentials not
// configured" because the SDK can't resolve credentials from a region.
// ---------------------------------------------------------------------------

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_bedrock_aws_region_alone_is_not_enough() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("AWS_REGION", "us-east-1");
    assert_eq!(
        detect_backend(),
        None,
        "AWS_REGION alone must not auto-select Bedrock"
    );
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_bedrock_static_creds() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("AWS_REGION", "us-east-1");
    g.set("AWS_ACCESS_KEY_ID", "AKIAFAKE");
    g.set("AWS_SECRET_ACCESS_KEY", "secret");
    assert_eq!(detect_backend().as_deref(), Some("bedrock"));
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_bedrock_access_key_without_secret_is_not_enough() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("AWS_REGION", "us-east-1");
    g.set("AWS_ACCESS_KEY_ID", "AKIAFAKE");
    // Missing AWS_SECRET_ACCESS_KEY — the SDK's env provider would fail.
    assert_eq!(detect_backend(), None);
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_bedrock_via_profile() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("AWS_PROFILE", "graphify");
    assert_eq!(detect_backend().as_deref(), Some("bedrock"));
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_bedrock_via_web_identity() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/sts/token");
    assert_eq!(detect_backend().as_deref(), Some("bedrock"));
}

#[test]
#[serial_test::serial(env)]
fn test_detect_backend_bedrock_via_ecs_relative_uri() {
    let mut g = EnvGuard::new();
    clear_backend_envs(&mut g);
    g.set(
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
        "/v2/credentials/abc",
    );
    assert_eq!(detect_backend().as_deref(), Some("bedrock"));
}

// ---------------------------------------------------------------------------
// Ollama num_ctx resolution (resolve_num_ctx)
// ---------------------------------------------------------------------------

#[test]
fn test_resolve_num_ctx_env_override() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_OLLAMA_NUM_CTX", "65536");

    let ctx = resolve_num_ctx("u", 8192);
    assert_eq!(ctx, 65536);
}

#[test]
fn test_resolve_num_ctx_auto_at_least_floor() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_OLLAMA_NUM_CTX");

    let ctx = resolve_num_ctx("u", 8192);
    assert!(ctx >= 8192, "auto num_ctx must be at least 8192; got {ctx}");
}

#[test]
fn test_resolve_num_ctx_scales_with_small_budget() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_OLLAMA_NUM_CTX");

    // ~8k input tokens = ~32k chars; with 16384 max_tokens the total should
    // be well under 131072.
    let small_chunk = "x".repeat(32_000);
    let ctx = resolve_num_ctx(&small_chunk, 16384);
    assert!(
        ctx < 131_072,
        "num_ctx={ctx} is too large for a small chunk; wastes VRAM"
    );
    assert!(ctx >= 8192);
}

#[test]
fn test_resolve_num_ctx_invalid_env_falls_back_to_auto() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_OLLAMA_NUM_CTX", "not-a-number");

    let ctx = resolve_num_ctx("u", 8192);
    // Should fall back to auto (at least floor)
    assert!(
        ctx >= 8192,
        "invalid env should fall back to auto; got {ctx}"
    );
}

// ---------------------------------------------------------------------------
// test_claude_cli_backend.py — MockRunner and call_claude_cli_with_runner
// ---------------------------------------------------------------------------

/// JSON envelope produced by `claude -p --output-format json`.
fn cli_envelope(result_inner: &str, stop_reason: &str, in_toks: u64, out_toks: u64) -> String {
    serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": result_inner,
        "stop_reason": stop_reason,
        "usage": {
            "input_tokens": in_toks,
            "output_tokens": out_toks,
            "cache_read_input_tokens": 17837_u64,
            "cache_creation_input_tokens": 30800_u64,
        },
        "modelUsage": {
            "claude-opus-4-7[1m]": {"inputTokens": in_toks, "outputTokens": out_toks}
        }
    })
    .to_string()
}

/// A successful extraction result embedded inside the envelope.
fn extraction_result_json() -> String {
    json!({
        "nodes": [
            {"id": "foo_module", "label": "Foo", "file_type": "document", "source_file": "foo.md"},
            {"id": "foo_greet",  "label": "greet", "file_type": "code",     "source_file": "foo.md"},
        ],
        "edges": [
            {"source": "foo_module", "target": "foo_greet",
             "relation": "references", "confidence": "EXTRACTED", "confidence_score": 1.0}
        ],
        "hyperedges": [],
        "input_tokens": 0,
        "output_tokens": 0,
    })
    .to_string()
}

/// Mock runner that returns a fixed stdout / code.
struct MockRunner {
    stdout: String,
    code: i32,
}

impl ClaudeRunner for MockRunner {
    fn run(&self, _user_message: &str, _append_system_prompt: bool) -> (String, String, i32) {
        (self.stdout.clone(), String::new(), self.code)
    }
}

/// Mock runner that records whether `--no-session-persistence` would have been sent.
/// We can't inspect the real process flags from the trait interface, but we can
/// verify the real `RealClaudeRunner` assembles the args by checking the trait
/// contract (the test in Python checks subprocess args; we verify the integration
/// at the `ClaudeCliBackend::new()` level).
#[test]
fn test_claude_cli_returns_parsed_nodes_and_edges() {
    let inner = extraction_result_json();
    let envelope = cli_envelope(&inner, "end_turn", 6, 11);
    let runner = MockRunner {
        stdout: envelope,
        code: 0,
    };

    // Bypass path check — inject runner directly.
    let result = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, true)
        .expect("should succeed");

    assert_eq!(result.nodes.len(), 2, "expected 2 nodes");
    assert_eq!(result.edges.len(), 1, "expected 1 edge");
}

#[test]
fn test_claude_cli_token_accounting_includes_cache() {
    let inner = extraction_result_json();
    let envelope = cli_envelope(&inner, "end_turn", 6, 11);
    let runner = MockRunner {
        stdout: envelope,
        code: 0,
    };

    let result = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, true)
        .expect("should succeed");

    // input_tokens = 6 + 17837 + 30800
    assert_eq!(
        result.input_tokens,
        6 + 17837 + 30800,
        "cache tokens must be included"
    );
    assert_eq!(result.output_tokens, 11);
    assert_eq!(result.model, "claude-opus-4-7[1m]");
    assert_eq!(result.finish_reason, "stop");
}

#[test]
fn test_claude_cli_finish_reason_length_on_max_tokens() {
    let inner = extraction_result_json();
    // Build the envelope with stop_reason="max_tokens"
    let envelope = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": inner,
        "stop_reason": "max_tokens",
        "usage": {
            "input_tokens": 6_u64,
            "output_tokens": 11_u64,
            "cache_read_input_tokens": 0_u64,
            "cache_creation_input_tokens": 0_u64,
        },
        "modelUsage": {"claude-opus-4-7[1m]": {"inputTokens": 6, "outputTokens": 11}}
    })
    .to_string();

    let runner = MockRunner {
        stdout: envelope,
        code: 0,
    };
    let result = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, true)
        .expect("should succeed");

    assert_eq!(result.finish_reason, "length");
}

#[test]
fn test_claude_cli_raises_on_nonzero_exit() {
    let runner = MockRunner {
        stdout: String::new(),
        code: 2,
    };
    let err = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, true)
        .expect_err("should fail on non-zero exit");

    let msg = err.to_string();
    assert!(
        msg.contains("exited 2"),
        "error should mention exit code: {msg}"
    );
}

#[test]
fn test_claude_cli_raises_on_garbage_envelope() {
    let runner = MockRunner {
        stdout: "not json".to_string(),
        code: 0,
    };
    let err = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, true)
        .expect_err("should fail on bad JSON");

    let msg = err.to_string();
    assert!(
        msg.contains("unparseable JSON envelope"),
        "error should mention JSON envelope: {msg}"
    );
}

#[test]
fn test_claude_cli_hollow_response_relabelled_as_length() {
    // An empty extraction result embedded in a successful envelope.
    let hollow_inner = json!({
        "nodes": [], "edges": [], "hyperedges": [],
        "input_tokens": 0, "output_tokens": 0
    })
    .to_string();
    let envelope = cli_envelope(&hollow_inner, "end_turn", 100, 0);
    let runner = MockRunner {
        stdout: envelope,
        code: 0,
    };

    let result = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, true)
        .expect("should succeed");

    assert_eq!(
        result.finish_reason, "length",
        "hollow response must be re-labelled 'length' so adaptive retry bisects"
    );
}

// ---------------------------------------------------------------------------
// Backend config sanity checks
// ---------------------------------------------------------------------------

#[test]
fn test_backend_config_default_models_non_empty() {
    for cfg in BACKENDS {
        assert!(
            !cfg.default_model.is_empty(),
            "backend '{}' has empty default_model",
            cfg.name
        );
    }
}

#[test]
fn test_router_returns_backend_for_all_registered_names() {
    // We only check that router() doesn't return UnknownBackend —
    // we don't call the backend (which would require credentials).
    let names = BACKENDS.iter().map(|b| b.name).collect::<Vec<_>>();
    for name in &names {
        let result = graphify_llm::router(name);
        assert!(
            result.is_ok(),
            "router({name:?}) should succeed; got: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_router_rejects_unknown_backend() {
    let result = graphify_llm::router("nonexistent-backend-xyz");
    let Err(err) = result else {
        panic!("expected error for nonexistent backend");
    };
    assert!(
        matches!(err, LlmError::UnknownBackend(..)),
        "expected UnknownBackend, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Chunk packing
// ---------------------------------------------------------------------------

#[test]
fn test_pack_chunks_by_tokens_single_chunk_for_small_files()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let paths: Vec<std::path::PathBuf> = (0..3)
        .map(|i| {
            let p = dir.path().join(format!("f{i}.md"));
            std::fs::write(&p, "hello").unwrap();
            p
        })
        .collect();

    let chunks = graphify_llm::pack_chunks_by_tokens(&paths, 100_000)?;
    assert_eq!(chunks.len(), 1, "small files should fit in one chunk");
    assert_eq!(chunks[0].len(), 3);
    Ok(())
}

#[test]
fn test_pack_chunks_by_tokens_rejects_zero_budget() {
    let paths: Vec<std::path::PathBuf> = vec![];
    let err = graphify_llm::pack_chunks_by_tokens(&paths, 0).expect_err("zero budget should fail");
    assert!(matches!(err, LlmError::InvalidInput(..)));
}
