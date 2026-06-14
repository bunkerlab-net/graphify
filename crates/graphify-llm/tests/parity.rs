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

#![allow(clippy::expect_used, clippy::float_cmp, unsafe_code)]

use graphify_llm::claude_cli::{ClaudeRunner, build_claude_cli_args, select_claude_command};
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
// test_llm_parser.py — _parse_llm_json robustness (PR #1062)
// ---------------------------------------------------------------------------

#[test]
fn test_parse_llm_json_preamble_then_fence_is_parsed() {
    // Claude often prefixes the JSON with a short preamble before the ```json
    // fence; the parser must handle a fence found anywhere, not only at offset 0.
    let raw = "Here are the extracted entities:\n\n```json\n{\"nodes\": [{\"id\": \"a\"}], \"edges\": []}\n```";
    let result = parse_llm_json(raw);
    assert_eq!(result["nodes"], json!([{"id": "a"}]));
    assert_eq!(result["edges"], json!([]));
}

#[test]
fn test_parse_llm_json_prose_wrapped_without_fence_is_parsed() {
    // Prose around bare JSON with no markdown fence: the balanced-brace
    // fallback extracts the first complete object.
    let raw =
        "The extracted graph is {\"nodes\": [{\"id\": \"b\"}], \"edges\": []}. Hope this helps!";
    let result = parse_llm_json(raw);
    assert_eq!(result["nodes"], json!([{"id": "b"}]));
}

#[test]
fn test_parse_llm_json_raw_json_still_works() {
    // Regression: clean JSON (the original happy path) must parse exactly.
    let raw = r#"{"nodes": [], "edges": [], "hyperedges": []}"#;
    let result = parse_llm_json(raw);
    assert_eq!(result, json!({"nodes": [], "edges": [], "hyperedges": []}));
}

#[test]
fn test_parse_llm_json_total_refusal_returns_empty_fragment() {
    // Model refusal / unrelated prose must degrade gracefully to the empty
    // fragment so the hollow detector takes over.
    let raw = "I cannot extract structured data from this content.";
    let result = parse_llm_json(raw);
    assert_eq!(result, json!({"nodes": [], "edges": [], "hyperedges": []}));
}

#[test]
fn test_parse_llm_json_fence_with_uppercase_language_tag() {
    let raw = "```JSON\n{\"nodes\": [{\"id\": \"x\"}], \"edges\": []}\n```";
    let result = parse_llm_json(raw);
    assert_eq!(result["nodes"], json!([{"id": "x"}]));
}

#[test]
fn test_parse_llm_json_fence_without_closing_backticks() {
    // Truncated response: model ran out of tokens before closing the fence.
    let raw = "```json\n{\"nodes\": [{\"id\": \"y\"}], \"edges\": []}";
    let result = parse_llm_json(raw);
    assert_eq!(result["nodes"], json!([{"id": "y"}]));
}

#[test]
fn test_parse_llm_json_empty_response_returns_empty_fragment() {
    assert_eq!(
        parse_llm_json(""),
        json!({"nodes": [], "edges": [], "hyperedges": []})
    );
}

#[test]
fn test_parse_llm_json_valid_json_with_fence_substring_is_not_mangled() {
    // A valid JSON payload whose string value contains a ``` substring must be
    // parsed verbatim. Stripping fences before the first parse would corrupt it
    // and lose the whole fragment.
    let raw = r#"{"nodes": [{"id": "x", "snippet": "```py\ncode\n```"}], "edges": []}"#;
    let result = parse_llm_json(raw);
    assert_eq!(result["nodes"][0]["snippet"], "```py\ncode\n```");
    assert_eq!(result["edges"], json!([]));
}

#[test]
fn test_parse_llm_json_skips_incidental_brace_before_real_json() {
    // An incidental, non-JSON brace group before the real payload must not abort
    // the scan: the parser keeps scanning balanced objects until one parses.
    let raw = "Note {see below}. Here is the graph: {\"nodes\": [{\"id\": \"z\"}], \"edges\": []}";
    let result = parse_llm_json(raw);
    assert_eq!(result["nodes"], json!([{"id": "z"}]));
    assert_eq!(result["edges"], json!([]));
}

#[test]
fn test_parse_llm_json_prefers_extraction_shaped_object() {
    // When an earlier brace group is itself valid JSON but not an extraction
    // fragment, the parser must skip it in favour of the object that carries
    // `nodes`/`edges`/`hyperedges`.
    let raw = "{\"status\": \"ok\"} then {\"nodes\": [], \"edges\": [{\"source\": \"a\"}]}";
    let result = parse_llm_json(raw);
    assert_eq!(result["edges"], json!([{"source": "a"}]));
    assert_eq!(result["nodes"], json!([]));
}

// ---------------------------------------------------------------------------
// test_llm_parser.py / test_claude_cli_backend.py — claude -p argv shape
// ---------------------------------------------------------------------------

#[test]
fn test_claude_cli_uses_system_prompt_not_append() {
    // The hollow-response root cause was --append-system-prompt layering
    // graphify's prompt on Claude Code's default; the fix switches to
    // --system-prompt (replace).
    let args = build_claude_cli_args(Some("SYSTEM"), None);
    assert!(
        args.iter().any(|a| a == "--system-prompt"),
        "--system-prompt missing from argv: {args:?}"
    );
    assert!(
        !args.iter().any(|a| a == "--append-system-prompt"),
        "--append-system-prompt should have been replaced"
    );
}

#[test]
fn test_claude_cli_model_arg_added_when_present() {
    // GRAPHIFY_CLAUDE_CLI_MODEL must be forwarded to `claude -p --model`.
    let args = build_claude_cli_args(Some("SYSTEM"), Some("haiku"));
    let idx = args
        .iter()
        .position(|a| a == "--model")
        .expect("--model flag present");
    assert_eq!(args[idx + 1], "haiku");
}

#[test]
fn test_claude_cli_no_model_arg_when_absent() {
    // Default behaviour: no --model so claude-cli's own default kicks in.
    let args = build_claude_cli_args(Some("SYSTEM"), None);
    assert!(!args.iter().any(|a| a == "--model"));
}

#[test]
fn test_claude_cli_blank_model_is_ignored() {
    // A blank/whitespace override (env var set to "") must not add --model.
    let args = build_claude_cli_args(Some("SYSTEM"), Some("   "));
    assert!(!args.iter().any(|a| a == "--model"));
}

#[test]
fn test_select_claude_windows_prefers_claude_cmd() {
    // On Windows, prefer the full path to claude.cmd over the unexecutable
    // claude.ps1 that a bare `claude` lookup resolves to (#1072).
    let chosen = select_claude_command(true, |name| match name {
        "claude" => Some(r"C:\npm\claude.ps1".to_string()),
        "claude.cmd" => Some(r"C:\npm\claude.cmd".to_string()),
        _ => None,
    });
    assert_eq!(chosen.as_deref(), Some(r"C:\npm\claude.cmd"));
}

#[test]
fn test_select_claude_windows_falls_back_to_bare_claude() {
    // claude.cmd missing but claude present (WSL-style): use the bare name.
    let chosen = select_claude_command(true, |name| {
        (name == "claude").then(|| "/usr/local/bin/claude".to_string())
    });
    assert_eq!(chosen.as_deref(), Some("claude"));
}

#[test]
fn test_select_claude_windows_none_when_neither_present() {
    let chosen = select_claude_command(true, |_| None);
    assert_eq!(chosen, None);
}

#[test]
fn test_select_claude_non_windows_uses_bare_claude() {
    let chosen = select_claude_command(false, |name| {
        (name == "claude").then(|| "/usr/local/bin/claude".to_string())
    });
    assert_eq!(chosen.as_deref(), Some("claude"));
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
#[serial_test::serial(env)]
fn test_resolve_num_ctx_env_override() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_OLLAMA_NUM_CTX", "65536");

    let ctx = resolve_num_ctx("u", 8192);
    assert_eq!(ctx, 65536);
}

#[test]
#[serial_test::serial(env)]
fn test_resolve_num_ctx_auto_at_least_floor() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_OLLAMA_NUM_CTX");

    let ctx = resolve_num_ctx("u", 8192);
    assert!(ctx >= 8192, "auto num_ctx must be at least 8192; got {ctx}");
}

#[test]
#[serial_test::serial(env)]
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
#[serial_test::serial(env)]
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
    fn run(
        &self,
        _user_message: &str,
        _system_prompt: Option<&str>,
        _model: Option<&str>,
        _timeout: std::time::Duration,
    ) -> (String, String, i32) {
        (self.stdout.clone(), String::new(), self.code)
    }
}

/// Runner that records the `timeout` it was handed, so we can assert
/// `GRAPHIFY_API_TIMEOUT` is threaded into the claude-cli call (#1112).
struct TimeoutRecordingRunner {
    seen: std::sync::Mutex<Option<std::time::Duration>>,
}

impl ClaudeRunner for TimeoutRecordingRunner {
    fn run(
        &self,
        _user_message: &str,
        _system_prompt: Option<&str>,
        _model: Option<&str>,
        timeout: std::time::Duration,
    ) -> (String, String, i32) {
        *self.seen.lock().expect("lock") = Some(timeout);
        // Minimal valid envelope so the caller parses without erroring.
        ("{\"result\": \"{}\"}".to_string(), String::new(), 0)
    }
}

/// Runner that records the `model` it was handed, so we can assert an explicit
/// model override reaches the claude-cli runner (#b304331).
struct ModelRecordingRunner {
    seen: std::sync::Mutex<Option<String>>,
}

impl ClaudeRunner for ModelRecordingRunner {
    fn run(
        &self,
        _user_message: &str,
        _system_prompt: Option<&str>,
        model: Option<&str>,
        _timeout: std::time::Duration,
    ) -> (String, String, i32) {
        *self.seen.lock().expect("lock") = model.map(str::to_string);
        ("{\"result\": \"{}\"}".to_string(), String::new(), 0)
    }
}

/// `call_claude_cli_inner` threads an explicit `--model` override into the runner.
#[test]
fn claude_cli_inner_threads_model_override() {
    let runner = ModelRecordingRunner {
        seen: std::sync::Mutex::new(None),
    };
    let _ = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        None,
        Some("haiku"),
    );
    assert_eq!(runner.seen.lock().expect("lock").as_deref(), Some("haiku"));
}

/// `test_claude_cli_extraction_honours_timeout`: the resolved
/// `GRAPHIFY_API_TIMEOUT` is passed through to the claude-cli runner.
#[test]
fn claude_cli_extraction_honours_timeout() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_API_TIMEOUT", "30");
    let runner = TimeoutRecordingRunner {
        seen: std::sync::Mutex::new(None),
    };
    let _ = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, None, None);
    assert_eq!(
        *runner.seen.lock().expect("lock"),
        Some(std::time::Duration::from_secs(30))
    );
}

/// Default (no env var) threads the 10-minute default through.
#[test]
fn claude_cli_extraction_default_timeout() {
    let mut g = EnvGuard::new();
    g.remove("GRAPHIFY_API_TIMEOUT");
    let runner = TimeoutRecordingRunner {
        seen: std::sync::Mutex::new(None),
    };
    let _ = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, None, None);
    assert_eq!(
        *runner.seen.lock().expect("lock"),
        Some(std::time::Duration::from_mins(10))
    );
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
    let result = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        Some(graphify_llm::EXTRACTION_SYSTEM),
        None,
    )
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

    let result = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        Some(graphify_llm::EXTRACTION_SYSTEM),
        None,
    )
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

/// CLI >= 2.1 emits a JSON ARRAY of streamed events terminated by a
/// `{"type":"result"}` object; it must parse identically to the legacy single
/// envelope (#edfe581).
#[test]
fn test_claude_cli_handles_json_array_envelope() {
    let inner = extraction_result_json();
    let result_obj: serde_json::Value =
        serde_json::from_str(&cli_envelope(&inner, "end_turn", 6, 11)).expect("result obj");
    let array = json!([
        {"type": "system", "subtype": "init"},
        {"type": "assistant", "message": {}},
        {"type": "rate_limit_event"},
        result_obj,
    ])
    .to_string();
    let runner = MockRunner {
        stdout: array,
        code: 0,
    };
    let result = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        Some(graphify_llm::EXTRACTION_SYSTEM),
        None,
    )
    .expect("array envelope should parse");
    assert_eq!(result.nodes.len(), 2);
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.input_tokens, 6 + 17837 + 30800);
}

/// A JSON array with no result object and a non-object tail is a hard error.
#[test]
fn test_claude_cli_array_without_result_object_errors() {
    let array = json!([{"type": "system"}, "not-an-object-tail"]).to_string();
    let runner = MockRunner {
        stdout: array,
        code: 0,
    };
    let err = graphify_llm::claude_cli::call_claude_cli_inner(&runner, "dummy", 8192, None, None)
        .expect_err("should error");
    match err {
        graphify_llm::LlmError::ClaudeCliError(m) => {
            assert!(m.contains("no result object"), "unexpected message: {m}");
        }
        other => panic!("expected ClaudeCliError, got {other:?}"),
    }
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
    let result = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        Some(graphify_llm::EXTRACTION_SYSTEM),
        None,
    )
    .expect("should succeed");

    assert_eq!(result.finish_reason, "length");
}

#[test]
fn test_claude_cli_raises_on_nonzero_exit() {
    let runner = MockRunner {
        stdout: String::new(),
        code: 2,
    };
    let err = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        Some(graphify_llm::EXTRACTION_SYSTEM),
        None,
    )
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
    let err = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        Some(graphify_llm::EXTRACTION_SYSTEM),
        None,
    )
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

    let result = graphify_llm::claude_cli::call_claude_cli_inner(
        &runner,
        "dummy",
        8192,
        Some(graphify_llm::EXTRACTION_SYSTEM),
        None,
    )
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
            std::fs::write(&p, "hello").expect("write fixture");
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

// ---------------------------------------------------------------------------
// --mode deep — extraction system prompt selection (graphify-py v0.8.22)
// ---------------------------------------------------------------------------

#[test]
fn test_extraction_system_non_deep_is_base_prompt() {
    let sys = graphify_llm::extraction_system(false);
    assert_eq!(sys.as_ref(), graphify_llm::EXTRACTION_SYSTEM);
    assert!(!sys.contains("DEEP_MODE"));
}

/// #cce2673: the extraction prompt states edge direction (source = actor) so
/// the model stops emitting reversed `calls` edges.
#[test]
fn test_extraction_system_states_edge_direction_rule() {
    let sys = graphify_llm::EXTRACTION_SYSTEM;
    assert!(sys.contains("Edge direction rule — source is always the ACTOR"));
    assert!(sys.contains("the function/method BEING CALLED. Never reverse this."));
}

#[test]
fn test_extraction_system_deep_appends_suffix() {
    let sys = graphify_llm::extraction_system(true);
    assert!(sys.starts_with(graphify_llm::EXTRACTION_SYSTEM));
    assert!(sys.ends_with(graphify_llm::DEEP_EXTRACTION_SUFFIX));
    assert!(sys.contains("DEEP_MODE: include additional INFERRED edges"));
    // Base prompt ends with a newline; the suffix opens with one, so the join
    // yields a blank-line separator (matches graphify-py concatenation).
    assert!(sys.contains("}\n\nDEEP_MODE"));
}
