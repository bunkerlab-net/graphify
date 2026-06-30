//! Per-backend mockito tests — each backend has a `GRAPHIFY_<NAME>_BASE_URL`
//! env var that, in combination with `GRAPHIFY_TEST_ALLOW_PRIVATE_IPS`, lets us
//! mock the HTTP endpoint.

#![allow(clippy::expect_used, clippy::float_cmp, unsafe_code)]

use graphify_llm::{
    call_llm,
    claude::call_claude,
    deepseek::{call_deepseek, call_deepseek_plain},
    gemini::{call_gemini, call_gemini_plain},
    kimi::call_kimi,
    openai::{call_openai, call_openai_plain},
};
use serde_json::json;

/// RAII guard that sets/restores multiple env vars.
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

fn allow_private() -> EnvGuard {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g
}

fn json_body(content: &str) -> String {
    json!({
        "choices": [{
            "message": {"content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 5, "completion_tokens": 8}
    })
    .to_string()
}

// ── openai ─────────────────────────────────────────────────────────────────

#[test]
fn openai_extraction_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let resp = call_openai(
        "key",
        "gpt-test",
        &[json!({"role":"user","content":"hi"})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
}

#[test]
fn openai_plain_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("plain answer"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let out = call_openai_plain("key", "gpt-test", "ping", 32).expect("test invariant");
    assert_eq!(out, "plain answer");
}

#[test]
#[serial_test::serial(env)]
fn openai_retries_rate_limited_request() {
    // A 429 must be retried (SDK max_retries parity, #1523): the first response is
    // a rate limit, the retry succeeds, so the call resolves instead of dropping
    // the chunk. Sequential mocks: 429 once, then 200.
    let mut server = mockito::Server::new();
    let _rl = server
        .mock("POST", "/chat/completions")
        .with_status(429)
        .with_body("rate limited")
        .expect(1)
        .create();
    let _ok = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"))
        .expect_at_least(1)
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("GRAPHIFY_MAX_RETRIES", "3");
    g.set("GRAPHIFY_RETRY_BASE_MS", "0"); // no backoff sleep under test

    let resp = call_openai(
        "key",
        "gpt-test",
        &[json!({"role":"user","content":"hi"})],
        128,
    )
    .expect("429 should be retried and then resolve");
    assert_eq!(resp.nodes.len(), 1);
}

#[test]
#[serial_test::serial(env)]
fn openai_gives_up_when_retries_disabled() {
    // GRAPHIFY_MAX_RETRIES=0 disables retries: a 429 fails immediately.
    let mut server = mockito::Server::new();
    let rl = server
        .mock("POST", "/chat/completions")
        .with_status(429)
        .with_body("rate limited")
        .expect(1)
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("GRAPHIFY_MAX_RETRIES", "0");
    g.set("GRAPHIFY_RETRY_BASE_MS", "0");

    assert!(
        call_openai(
            "key",
            "gpt-test",
            &[json!({"role":"user","content":"hi"})],
            128
        )
        .is_err(),
        "with retries disabled, a 429 must fail"
    );
    rl.assert(); // exactly one request: GRAPHIFY_MAX_RETRIES=0 means no retry
}

// ── gemini ─────────────────────────────────────────────────────────────────

#[test]
fn gemini_extraction_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("{\"nodes\":[{\"id\":\"y\"}],\"edges\":[]}"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_GEMINI_BASE_URL", &server.url());

    let resp = call_gemini(
        "key",
        "gemini-test",
        &[json!({"role":"user","content":"hi"})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
}

#[test]
fn gemini_plain_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("gemini answer"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_GEMINI_BASE_URL", &server.url());

    let out = call_gemini_plain("key", "gemini-test", "ping", 32).expect("test invariant");
    assert_eq!(out, "gemini answer");
}

// ── kimi ───────────────────────────────────────────────────────────────────

#[test]
fn kimi_extraction_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("{\"nodes\":[{\"id\":\"z\"}],\"edges\":[]}"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_KIMI_BASE_URL", &server.url());

    let resp = call_kimi(
        "key",
        "kimi-test",
        &[json!({"role":"user","content":"hi"})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
}

// ── deepseek ───────────────────────────────────────────────────────────────

#[test]
fn deepseek_extraction_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("{\"nodes\":[{\"id\":\"d\"}],\"edges\":[]}"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_DEEPSEEK_BASE_URL", &server.url());

    let resp = call_deepseek(
        "key",
        "deepseek-test",
        &[json!({"role":"user","content":"hi"})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
}

#[test]
fn deepseek_plain_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("deepseek answer"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_DEEPSEEK_BASE_URL", &server.url());

    let out = call_deepseek_plain("key", "ds-test", "ping", 32).expect("test invariant");
    assert_eq!(out, "deepseek answer");
}

// ── claude (direct anthropic API) ──────────────────────────────────────────

#[test]
fn claude_extraction_via_mock() {
    let mut server = mockito::Server::new();
    let body = json!({
        "content": [{"text": "{\"nodes\":[{\"id\":\"c\"}],\"edges\":[]}"}],
        "usage": {"input_tokens": 4, "output_tokens": 9},
        "stop_reason": "end_turn"
    });
    let _m = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_CLAUDE_BASE_URL", &server.url());

    let resp = call_claude(
        "key",
        "claude-test",
        &[json!({"role":"user","content":"hi"})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
    assert_eq!(resp.input_tokens, 4);
    assert_eq!(resp.output_tokens, 9);
}

#[test]
fn claude_max_tokens_stop_reason_maps_to_length() {
    let mut server = mockito::Server::new();
    let body = json!({
        "content": [{"text": "{\"nodes\":[{\"id\":\"c\"}],\"edges\":[]}"}],
        "usage": {"input_tokens": 100, "output_tokens": 128},
        "stop_reason": "max_tokens"
    });
    let _m = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_CLAUDE_BASE_URL", &server.url());

    let resp = call_claude(
        "key",
        "claude-test",
        &[json!({"role":"user","content":"hi"})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.finish_reason, "length");
}

// ── call_llm dispatcher with mocks for several backends ───────────────────

#[test]
fn call_llm_dispatch_openai_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("dispatched"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("OPENAI_API_KEY", "test-key");

    let out = call_llm("hi", "openai", 32).expect("test invariant");
    assert_eq!(out, "dispatched");
}

#[test]
fn call_llm_dispatch_gemini_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("gemini response"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_GEMINI_BASE_URL", &server.url());
    g.set("GEMINI_API_KEY", "test-key");

    let out = call_llm("hi", "gemini", 32).expect("test invariant");
    assert_eq!(out, "gemini response");
}

#[test]
fn call_llm_dispatch_kimi_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("kimi response"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_KIMI_BASE_URL", &server.url());
    g.set("MOONSHOT_API_KEY", "test-key");

    let out = call_llm("hi", "kimi", 32).expect("test invariant");
    assert_eq!(out, "kimi response");
}

#[test]
fn call_llm_dispatch_deepseek_via_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(json_body("deepseek response"))
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_DEEPSEEK_BASE_URL", &server.url());
    g.set("DEEPSEEK_API_KEY", "test-key");

    let out = call_llm("hi", "deepseek", 32).expect("test invariant");
    assert_eq!(out, "deepseek response");
}

#[test]
fn call_llm_dispatch_claude_via_mock() {
    let mut server = mockito::Server::new();
    let body = json!({
        "content": [{"text": "claude says hi"}],
        "usage": {"input_tokens": 1, "output_tokens": 4},
        "stop_reason": "end_turn"
    });
    let _m = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = allow_private();
    g.set("GRAPHIFY_CLAUDE_BASE_URL", &server.url());
    g.set("ANTHROPIC_API_KEY", "test-key");

    let out = call_llm("hi", "claude", 32).expect("test invariant");
    assert!(out.contains("claude"));
}
