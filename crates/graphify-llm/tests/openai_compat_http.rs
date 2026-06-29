//! Mockito-driven tests for `call_openai_compat` covering happy path,
//! parse errors, hollow-response detection, and finish-reason handling.

#![allow(clippy::expect_used, clippy::float_cmp, unsafe_code)]

use std::time::Duration;

use graphify_llm::openai_compat::{OllamaOptions, OpenAiRequest, call_openai_compat};
use serde_json::json;

/// Guard that enables the SSRF private-IP bypass via env var for the duration
/// of one test, then restores the prior state on drop.
struct AllowPrivate {
    prev: Option<String>,
}

impl AllowPrivate {
    fn new() -> Self {
        let prev = std::env::var("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS").ok();
        // SAFETY: test-only.
        unsafe { std::env::set_var("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1") };
        Self { prev }
    }
}

impl Drop for AllowPrivate {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => unsafe { std::env::set_var("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", v) },
            None => unsafe { std::env::remove_var("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS") },
        }
    }
}

fn make_req<'a>(base_url: &'a str, backend_name: &'a str) -> OpenAiRequest<'a> {
    OpenAiRequest {
        base_url,
        api_key: "test-key",
        model: "test-model",
        messages: vec![json!({"role": "user", "content": "hi"})],
        temperature: Some(0.0),
        reasoning_effort: None,
        max_completion_tokens: 256,
        disable_thinking: false,
        custom_extra_body: None,
        ollama_options: None,
        backend_name,
        timeout: Duration::from_secs(5),
    }
}

// ── happy path ─────────────────────────────────────────────────────────────

#[test]
fn call_openai_compat_happy_path() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"a\"}],\"edges\":[]}"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 7, "completion_tokens": 12}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let req = make_req(&url, "openai");
    let resp = call_openai_compat(&req).expect("happy path should succeed");
    assert_eq!(resp.input_tokens, 7);
    assert_eq!(resp.output_tokens, 12);
    assert_eq!(resp.finish_reason, "stop");
    assert_eq!(resp.nodes.len(), 1);
}

/// #1223: the chat-completion request must carry `stream: false` so SSE-default
/// gateways return a single response. The mock only matches when the body
/// contains `stream: false`; a missing field makes the mock 501 and the call
/// fails, so a green call proves the field is present.
#[test]
fn call_openai_compat_forces_non_streaming() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{"message": {"content": "{\"nodes\":[],\"edges\":[]}"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(json!({"stream": false})))
        .with_status(200)
        .with_header("Content-Type", "application/json")
        .with_body(body.to_string())
        .create();
    let url = server.url();
    let req = make_req(&url, "openai");
    call_openai_compat(&req).expect("request body must carry stream:false");
}

// ── hollow response → reclassified as "length" ─────────────────────────────

#[test]
fn call_openai_compat_hollow_response_becomes_length() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": ""},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 5, "completion_tokens": 0}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let req = make_req(&url, "openai");
    let resp = call_openai_compat(&req).expect("test invariant");
    assert_eq!(
        resp.finish_reason, "length",
        "hollow content should be reclassified"
    );
}

// ── empty choices array → EmptyResponse error ──────────────────────────────

#[test]
fn call_openai_compat_empty_choices_errors() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({"choices": [], "usage": {}});
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let req = make_req(&url, "openai");
    let err = call_openai_compat(&req).expect_err("empty choices should fail");
    assert!(format!("{err}").contains("no choices"));
}

// ── server returns malformed JSON → Parse error ───────────────────────────

#[test]
fn call_openai_compat_invalid_json_errors() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body("not valid json")
        .create();

    let url = server.url();
    let req = make_req(&url, "openai");
    let err = call_openai_compat(&req).expect_err("invalid JSON should fail");
    assert!(
        format!("{err:?}").to_lowercase().contains("parse")
            || format!("{err:?}").to_lowercase().contains("http")
    );
}

// ── server returns 5xx → Http error ───────────────────────────────────────

#[test]
fn call_openai_compat_500_errors() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(500)
        .with_body("server boom")
        .create();

    let url = server.url();
    let req = make_req(&url, "openai");
    assert!(call_openai_compat(&req).is_err());
}

// ── finish_reason="length" preserved on real truncation ───────────────────

#[test]
fn call_openai_compat_real_truncation_preserved() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"a\"}],\"edges\":[]}"},
            "finish_reason": "length"
        }],
        "usage": {"prompt_tokens": 100, "completion_tokens": 256}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let req = make_req(&url, "openai");
    let resp = call_openai_compat(&req).expect("test invariant");
    assert_eq!(resp.finish_reason, "length");
}

// ── disable_thinking flag is forwarded in request body ────────────────────

#[test]
fn call_openai_compat_with_disable_thinking() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{"message": {"content": "{\"nodes\":[],\"edges\":[]}"}, "finish_reason": "stop"}],
        "usage": {}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJsonString(
            "{\"extra_body\":{\"thinking\":{\"type\":\"disabled\"}}}".into(),
        ))
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let mut req = make_req(&url, "kimi");
    req.disable_thinking = true;
    let _ = call_openai_compat(&req).expect("test invariant");
}

// ── ollama_options forwarded as extra_body ────────────────────────────────

#[test]
fn call_openai_compat_with_ollama_options() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{"message": {"content": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"}, "finish_reason": "stop"}],
        "usage": {"completion_tokens": 100}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let mut req = make_req(&url, "ollama");
    req.ollama_options = Some(OllamaOptions {
        num_ctx: 8192,
        keep_alive: "5m".to_string(),
    });
    let resp = call_openai_compat(&req).expect("test invariant");
    assert_eq!(resp.finish_reason, "stop");
}

// ── custom-provider extra_body passthrough (#7477b46) ─────────────────────

#[test]
fn call_openai_compat_uses_explicit_extra_body() {
    // A custom provider's extra_body is forwarded verbatim — lets self-hosted
    // Qwen3 on vLLM pass `chat_template_kwargs.enable_thinking=false`.
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{"message": {"content": "{\"nodes\":[],\"edges\":[]}"}, "finish_reason": "stop"}],
        "usage": {}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJsonString(
            "{\"extra_body\":{\"chat_template_kwargs\":{\"enable_thinking\":false}}}".into(),
        ))
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let eb = json!({"chat_template_kwargs": {"enable_thinking": false}});
    let mut req = make_req(&url, "kitor-vllm");
    req.custom_extra_body = Some(&eb);
    let _ = call_openai_compat(&req).expect("test invariant");
}

#[test]
fn call_openai_compat_extra_body_wins_over_moonshot_default() {
    // disable_thinking would inject `thinking: disabled`, but an explicit
    // custom extra_body must override it.
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{"message": {"content": "{\"nodes\":[],\"edges\":[]}"}, "finish_reason": "stop"}],
        "usage": {}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJsonString(
            "{\"extra_body\":{\"thinking\":{\"type\":\"enabled\"}}}".into(),
        ))
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let eb = json!({"thinking": {"type": "enabled"}});
    let mut req = make_req(&url, "kimi");
    req.disable_thinking = true;
    req.custom_extra_body = Some(&eb);
    let _ = call_openai_compat(&req).expect("test invariant");
}

#[test]
fn call_openai_compat_explicit_extra_body_skips_ollama_auto_derive() {
    // An explicit extra_body means "I own this request shape" — Ollama's
    // num_ctx auto-derive must step aside, not clobber it.
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{"message": {"content": "{\"nodes\":[],\"edges\":[]}"}, "finish_reason": "stop"}],
        "usage": {}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJsonString(
            "{\"extra_body\":{\"options\":{\"num_ctx\":4096}}}".into(),
        ))
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let eb = json!({"options": {"num_ctx": 4096}});
    let mut req = make_req(&url, "ollama");
    req.ollama_options = Some(OllamaOptions {
        num_ctx: 65536,
        keep_alive: "30m".to_string(),
    });
    req.custom_extra_body = Some(&eb);
    let _ = call_openai_compat(&req).expect("test invariant");
}

// ── reasoning_effort forwarded ─────────────────────────────────────────────

#[test]
fn call_openai_compat_with_reasoning_effort() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{"message": {"content": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"}, "finish_reason": "stop"}],
        "usage": {}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let mut req = make_req(&url, "openai");
    req.reasoning_effort = Some("low");
    let _ = call_openai_compat(&req).expect("test invariant");
}

// ── Each public per-backend wrapper exercised once via call_openai_compat ─

#[test]
fn call_gemini_via_compat_works_with_mock() {
    let _g = AllowPrivate::new();
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"},
            "finish_reason": "stop"
        }],
        "usage": {}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let url = server.url();
    let req = make_req(&url, "gemini");
    let resp = call_openai_compat(&req).expect("test invariant");
    assert_eq!(resp.finish_reason, "stop");
    assert_eq!(resp.nodes.len(), 1);
}
