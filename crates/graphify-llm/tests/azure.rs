//! Tests for the Azure `OpenAI` backend.
//!
//! Mirrors the azure cases in `graphify-py/tests/test_llm_backends.py`
//! (`test_call_azure_*`). Env-var tests use a scoped guard; the live call is
//! exercised against a mockito server with the SSRF private-IP bypass.

#![allow(clippy::expect_used, unsafe_code)]

use graphify_llm::azure;
use serde_json::json;
use serial_test::serial;

/// Scoped env guard: set/remove keys, restore on drop.
struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self { saved: vec![] }
    }
    fn set(&mut self, key: &str, value: &str) -> &mut Self {
        self.saved.push((key.to_string(), std::env::var(key).ok()));
        // SAFETY: test-only, serialized via #[serial].
        unsafe { std::env::set_var(key, value) };
        self
    }
    fn remove(&mut self, key: &str) -> &mut Self {
        self.saved.push((key.to_string(), std::env::var(key).ok()));
        // SAFETY: test-only, serialized via #[serial].
        unsafe { std::env::remove_var(key) };
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

#[test]
#[serial(env)]
fn resolve_model_prefers_deployment_then_model_env_then_default() {
    let mut g = EnvGuard::new();
    g.remove("AZURE_OPENAI_DEPLOYMENT");
    g.remove("GRAPHIFY_AZURE_MODEL");
    assert_eq!(azure::resolve_model(), "gpt-4o");

    g.set("GRAPHIFY_AZURE_MODEL", "gpt-4o-mini");
    assert_eq!(azure::resolve_model(), "gpt-4o-mini");

    // Deployment wins over the model-override env var.
    g.set("AZURE_OPENAI_DEPLOYMENT", "my-deploy");
    assert_eq!(azure::resolve_model(), "my-deploy");
}

#[test]
#[serial(env)]
fn resolve_api_version_honours_env_then_default() {
    let mut g = EnvGuard::new();
    g.remove("AZURE_OPENAI_API_VERSION");
    assert_eq!(azure::resolve_api_version(), "2024-12-01-preview");

    g.set("AZURE_OPENAI_API_VERSION", "2024-08-01-preview");
    assert_eq!(azure::resolve_api_version(), "2024-08-01-preview");
}

#[test]
#[serial(env)]
fn chat_url_is_deployment_scoped_with_api_version() {
    let mut g = EnvGuard::new();
    g.set("AZURE_OPENAI_API_VERSION", "2024-08-01-preview");
    let url = azure::chat_url("https://my-resource.openai.azure.com/", "gpt-4o");
    assert_eq!(
        url,
        "https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-08-01-preview"
    );
}

#[test]
#[serial(env)]
fn resolve_endpoint_errors_when_unset() {
    let mut g = EnvGuard::new();
    g.remove("AZURE_OPENAI_ENDPOINT");
    let err = azure::resolve_endpoint().expect_err("must error when endpoint unset");
    assert!(err.to_string().contains("AZURE_OPENAI_ENDPOINT"));
}

#[test]
#[serial(env)]
fn call_azure_posts_to_deployment_url_with_api_key_header() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("AZURE_OPENAI_API_VERSION", "2024-08-01-preview");

    let mut server = mockito::Server::new();
    let mock = server
        .mock(
            "POST",
            "/openai/deployments/gpt-4o/chat/completions?api-version=2024-08-01-preview",
        )
        .match_header("api-key", "test-key")
        // Azure must use max_completion_tokens, never the deprecated max_tokens.
        .match_body(mockito::Matcher::AllOf(vec![
            mockito::Matcher::PartialJsonString("{\"max_completion_tokens\":8192}".into()),
        ]))
        .with_status(200)
        .with_body(
            json!({
                "choices": [{
                    "message": {"content": "{\"nodes\":[{\"id\":\"a\"}],\"edges\":[],\"hyperedges\":[]}"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 100, "completion_tokens": 50}
            })
            .to_string(),
        )
        .create();

    let messages = vec![json!({"role": "user", "content": "test"})];
    let resp = azure::call_azure("test-key", &server.url(), "gpt-4o", &messages, 8192)
        .expect("azure call succeeds");

    assert_eq!(resp.nodes, vec![json!({"id": "a"})]);
    assert_eq!(resp.input_tokens, 100);
    assert_eq!(resp.output_tokens, 50);
    assert_eq!(resp.finish_reason, "stop");
    assert_eq!(resp.model, "gpt-4o");
    mock.assert();
}

#[test]
#[serial(env)]
fn call_azure_plain_returns_content() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.remove("AZURE_OPENAI_API_VERSION");

    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", mockito::Matcher::Any)
        .match_header("api-key", "tk")
        .with_status(200)
        .with_body(
            json!({
                "choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}],
                "usage": {}
            })
            .to_string(),
        )
        .create();

    let out = azure::call_azure_plain("tk", &server.url(), "gpt-4o", "hi", 64)
        .expect("plain call succeeds");
    assert_eq!(out, "hello");
    mock.assert();
}
