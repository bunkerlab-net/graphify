//! Bedrock mockito tests. Honours `GRAPHIFY_BEDROCK_BASE_URL` so the regional
//! AWS endpoint can be replaced with a local mock server.

#![allow(clippy::expect_used, unsafe_code)]

use graphify_llm::bedrock::{call_bedrock, call_bedrock_plain, resolve_region};
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

// ── resolve_region ─────────────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn resolve_region_defaults_to_us_east_1() {
    let mut g = EnvGuard::new();
    g.remove("AWS_REGION");
    g.remove("AWS_DEFAULT_REGION");
    assert_eq!(resolve_region(), "us-east-1");
}

#[test]
#[serial_test::serial(env)]
fn resolve_region_uses_aws_region() {
    let mut g = EnvGuard::new();
    g.set("AWS_REGION", "eu-west-2");
    assert_eq!(resolve_region(), "eu-west-2");
}

#[test]
#[serial_test::serial(env)]
fn resolve_region_falls_back_to_aws_default_region() {
    let mut g = EnvGuard::new();
    g.remove("AWS_REGION");
    g.set("AWS_DEFAULT_REGION", "ap-south-1");
    assert_eq!(resolve_region(), "ap-south-1");
}

// ── call_bedrock missing credentials ───────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn call_bedrock_missing_creds_errors() {
    let mut g = EnvGuard::new();
    g.remove("AWS_ACCESS_KEY_ID");
    g.remove("AWS_SECRET_ACCESS_KEY");
    let result = call_bedrock(
        "test-model",
        "us-east-1",
        &[json!({"role": "user", "content": [{"text": "hi"}]})],
        128,
    );
    assert!(result.is_err());
}

// ── call_bedrock happy path via mock ───────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn call_bedrock_via_mock() {
    let mut server = mockito::Server::new();
    let body = json!({
        "output": {
            "message": {
                "content": [{"text": "{\"nodes\":[{\"id\":\"b\"}],\"edges\":[]}"}]
            }
        },
        "usage": {"inputTokens": 3, "outputTokens": 5},
        "stopReason": "end_turn"
    });
    let _m = server
        .mock("POST", mockito::Matcher::Any)
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_BEDROCK_BASE_URL", &server.url());
    g.set("AWS_ACCESS_KEY_ID", "fake-access");
    g.set("AWS_SECRET_ACCESS_KEY", "fake-secret");

    let resp = call_bedrock(
        "test-model",
        "us-east-1",
        &[json!({"role": "user", "content": [{"text": "hi"}]})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
    assert_eq!(resp.input_tokens, 3);
    assert_eq!(resp.output_tokens, 5);
    assert_eq!(resp.finish_reason, "stop");
}

#[test]
#[serial_test::serial(env)]
fn call_bedrock_max_tokens_maps_to_length() {
    let mut server = mockito::Server::new();
    let body = json!({
        "output": {
            "message": {
                "content": [{"text": "{\"nodes\":[{\"id\":\"b\"}],\"edges\":[]}"}]
            }
        },
        "usage": {"inputTokens": 100, "outputTokens": 128},
        "stopReason": "max_tokens"
    });
    let _m = server
        .mock("POST", mockito::Matcher::Any)
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_BEDROCK_BASE_URL", &server.url());
    g.set("AWS_ACCESS_KEY_ID", "fake-access");
    g.set("AWS_SECRET_ACCESS_KEY", "fake-secret");

    let resp = call_bedrock(
        "test-model",
        "us-east-1",
        &[json!({"role": "user", "content": [{"text": "hi"}]})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.finish_reason, "length");
}

#[test]
#[serial_test::serial(env)]
fn call_bedrock_with_session_token() {
    let mut server = mockito::Server::new();
    let body = json!({
        "output": {"message": {"content": [{"text": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"}]}},
        "usage": {"inputTokens": 1, "outputTokens": 2}
    });
    let _m = server
        .mock("POST", mockito::Matcher::Any)
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_BEDROCK_BASE_URL", &server.url());
    g.set("AWS_ACCESS_KEY_ID", "fake");
    g.set("AWS_SECRET_ACCESS_KEY", "fake");
    g.set("AWS_SESSION_TOKEN", "fake-session");

    let resp = call_bedrock(
        "test-model",
        "us-east-1",
        &[json!({"role": "user", "content": [{"text": "hi"}]})],
        128,
    )
    .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
}

#[test]
#[serial_test::serial(env)]
fn call_bedrock_plain_via_mock() {
    let mut server = mockito::Server::new();
    let body = json!({
        "output": {"message": {"content": [{"text": "the answer"}]}},
        "usage": {"inputTokens": 1, "outputTokens": 2}
    });
    let _m = server
        .mock("POST", mockito::Matcher::Any)
        .with_status(200)
        .with_body(body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_BEDROCK_BASE_URL", &server.url());
    g.set("AWS_ACCESS_KEY_ID", "fake");
    g.set("AWS_SECRET_ACCESS_KEY", "fake");

    let _ = call_bedrock_plain("test-model", "us-east-1", "hi", 32).expect("test invariant");
    // call_bedrock_plain returns the first nodes[].label string, which may be
    // empty since the JSON we returned isn't extraction-shaped — just verify
    // the function runs without panicking.
}

// ── 5xx error path ─────────────────────────────────────────────────────────

#[test]
#[serial_test::serial(env)]
fn call_bedrock_http_error() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", mockito::Matcher::Any)
        .with_status(500)
        .with_body("boom")
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_BEDROCK_BASE_URL", &server.url());
    g.set("AWS_ACCESS_KEY_ID", "fake");
    g.set("AWS_SECRET_ACCESS_KEY", "fake");

    let result = call_bedrock(
        "test-model",
        "us-east-1",
        &[json!({"role": "user", "content": [{"text": "hi"}]})],
        128,
    );
    assert!(result.is_err());
}
