//! Ollama mockito tests.

#![allow(clippy::expect_used, unsafe_code)]

use std::net::IpAddr;

use graphify_llm::ollama::{
    call_ollama, call_ollama_plain, ollama_host_is_link_local_or_metadata_with,
    validate_ollama_base_url,
};
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
    // Loopback, a legit remote host, and an unparseable string all succeed
    // (warnings only) — none of these is a link-local / metadata target.
    assert!(validate_ollama_base_url("http://localhost:11434/v1", true).is_ok());
    assert!(validate_ollama_base_url("https://remote-ollama.example.com", true).is_ok());
    assert!(validate_ollama_base_url("not-a-url-at-all", true).is_ok());
}

// ── F3: link-local / cloud-metadata hard-block (port of test_ollama.py) ──────

#[test]
fn ollama_blocks_link_local_and_metadata() {
    // Link-local / cloud-metadata Ollama targets fail closed (F3).
    for url in [
        "http://169.254.169.254/v1",
        "http://169.254.1.5:11434/v1",
        "http://metadata.google.internal/v1",
        "http://0.0.0.0:11434/v1",
        // Bracketed IPv6 link-local literal (fe80::/10) — brackets are stripped
        // before the IP check, matching Python's `urlparse().hostname`.
        "http://[fe80::1]/v1",
    ] {
        assert!(
            validate_ollama_base_url(url, true).is_err(),
            "{url} should be blocked"
        );
    }
}

#[test]
fn ollama_loopback_and_lan_do_not_raise() {
    // Loopback is allowed silently; a general LAN host warns but is allowed.
    // (Rust emits the warning via eprintln; the test asserts the allow/block
    // outcome rather than capturing stderr.)
    assert!(validate_ollama_base_url("http://localhost:11434/v1", true).is_ok());
    assert!(validate_ollama_base_url("http://192.168.1.50:11434/v1", true).is_ok());
    // Bracketed IPv6 loopback is allowed (brackets stripped → "::1").
    assert!(validate_ollama_base_url("http://[::1]:11434/v1", true).is_ok());
}

#[test]
fn ollama_alias_resolving_to_link_local_blocked() {
    // A hostname that RESOLVES to a link-local IP is caught, not just literals.
    // The resolver is injected here (Python monkeypatches socket.getaddrinfo).
    let resolves_to_metadata = |_host: &str| vec![IpAddr::from([169, 254, 169, 254])];
    assert!(ollama_host_is_link_local_or_metadata_with(
        "innocent-looking-host",
        resolves_to_metadata
    ));
    // A host resolving to a normal public IP is not flagged.
    let resolves_to_public = |_host: &str| vec![IpAddr::from([93, 184, 216, 34])];
    assert!(!ollama_host_is_link_local_or_metadata_with(
        "example.com",
        resolves_to_public
    ));
    // A host resolving to an IPv6 link-local (fe80::/10) is also caught.
    let resolves_to_v6_ll = |_host: &str| vec![IpAddr::from([0xfe80, 0, 0, 0, 0, 0, 0, 1])];
    assert!(ollama_host_is_link_local_or_metadata_with(
        "v6-host",
        resolves_to_v6_ll
    ));
}

#[test]
fn ollama_warn_false_still_hard_blocks() {
    // warn=false suppresses the LAN warning but never the metadata hard-block.
    assert!(validate_ollama_base_url("http://192.168.1.50:11434/v1", false).is_ok());
    assert!(validate_ollama_base_url("http://169.254.169.254/v1", false).is_err());
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
