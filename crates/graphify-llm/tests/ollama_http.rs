//! Ollama mockito tests.

#![allow(clippy::expect_used, unsafe_code)]

use graphify_llm::ollama::{call_ollama, call_ollama_plain, validate_ollama_base_url};
use serde_json::json;

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

#[test]
fn validate_ollama_base_url_doesnt_panic() {
    // The validator just logs warnings — should not panic on any input.
    validate_ollama_base_url("http://localhost:11434/v1");
    validate_ollama_base_url("https://remote-ollama.example.com");
    validate_ollama_base_url("not-a-url-at-all");
}

#[test]
#[serial_test::serial(env)]
fn call_ollama_via_mock() {
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"o\"}],\"edges\":[]}"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 12, "completion_tokens": 100}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let resp = call_ollama(
        "ollama",
        &server.url(),
        "llama-test",
        &[json!({"role": "user", "content": "hello world"})],
        128,
        "hello world",
    )
    .expect("mock ollama server should return a valid graph response");
    assert_eq!(resp.nodes.len(), 1);
    assert_eq!(resp.input_tokens, 12);
}

#[test]
#[serial_test::serial(env)]
fn call_ollama_plain_via_mock() {
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": "ollama answers"},
            "finish_reason": "stop"
        }]
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let out = call_ollama_plain("ollama", &server.url(), "llama-test", "ping", 32)
        .expect("mock ollama server should return a plain-text response");
    assert_eq!(out, "ollama answers");
}

#[test]
#[serial_test::serial(env)]
fn call_ollama_low_token_warning_path() {
    // When output_tokens < 50, the helper emits a stderr warning.
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 5, "completion_tokens": 10}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let resp = call_ollama(
        "ollama",
        &server.url(),
        "tiny-model",
        &[json!({"role": "user", "content": "hi"})],
        64,
        "hi",
    )
    .expect("mock ollama server should return a response with low token count");
    assert_eq!(resp.output_tokens, 10);
}

#[test]
#[serial_test::serial(env)]
fn call_ollama_http_error() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(500)
        .with_body("internal")
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");

    let result = call_ollama(
        "ollama",
        &server.url(),
        "llama-test",
        &[json!({"role": "user", "content": "hi"})],
        128,
        "hi",
    );
    assert!(result.is_err());
}
