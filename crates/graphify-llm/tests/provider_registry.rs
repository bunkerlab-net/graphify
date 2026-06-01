//! Parity tests for the custom LLM provider registry (#1084).
//!
//! Mirrors `graphify-py/tests/test_provider_registry.py`.
#![allow(clippy::expect_used, clippy::float_cmp, unsafe_code)]

use graphify_llm::{
    CustomProvider, Pricing, call_llm, detect_backend_with, extract_files_direct,
    load_custom_providers_from,
};
use indexmap::IndexMap;
use serial_test::serial;
use tempfile::tempdir;

mod common;
use common::EnvGuard;

#[test]
fn custom_provider_load_returns_config() {
    let tmp = tempdir().expect("tempdir");
    let global = tmp.path().join("providers.json");
    std::fs::write(
        &global,
        r#"{
            "nvidia": {
                "base_url": "https://integrate.api.nvidia.com/v1",
                "default_model": "minimaxai/minimax-m2.7",
                "env_key": "NVIDIA_API_KEY",
                "pricing": {"input": 0.0, "output": 0.0},
                "temperature": 0
            }
        }"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&tmp.path().join("local.json"), &global);
    assert!(loaded.contains_key("nvidia"));
    assert_eq!(
        loaded["nvidia"].base_url,
        "https://integrate.api.nvidia.com/v1"
    );
}

#[test]
fn custom_provider_identical_local_global_path_read_once() {
    // When `$HOME` is unset the local and global paths coincide; the registry
    // must still be read only once (no double-insert / wasted read).
    let tmp = tempdir().expect("tempdir");
    let path = tmp.path().join("providers.json");
    std::fs::write(
        &path,
        r#"{"only": {"base_url": "http://x/v1", "default_model": "m", "env_key": "K"}}"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&path, &path);
    assert_eq!(loaded.len(), 1);
    assert!(loaded.contains_key("only"));
}

#[test]
fn custom_provider_max_completion_tokens_parsed_and_defaulted() {
    // A provider may set `max_completion_tokens` (honoured on the extraction
    // path, mirroring Python's `cfg.get("max_completion_tokens", 8192)`); when
    // omitted it falls back to 8192.
    let tmp = tempdir().expect("tempdir");
    let global = tmp.path().join("providers.json");
    std::fs::write(
        &global,
        r#"{
            "big": {"base_url": "http://x/v1", "default_model": "m", "env_key": "K", "max_completion_tokens": 16000},
            "flt": {"base_url": "http://z/v1", "default_model": "m", "env_key": "K", "max_completion_tokens": 12000.0},
            "neg": {"base_url": "http://w/v1", "default_model": "m", "env_key": "K", "max_completion_tokens": -5},
            "dflt": {"base_url": "http://y/v1", "default_model": "m", "env_key": "K"}
        }"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&tmp.path().join("local.json"), &global);
    assert_eq!(loaded["big"].max_completion_tokens, 16000);
    // A whole-number JSON float is accepted...
    assert_eq!(loaded["flt"].max_completion_tokens, 12000);
    // ...but a negative value falls back to the default rather than wrapping.
    assert_eq!(loaded["neg"].max_completion_tokens, 8192);
    assert_eq!(loaded["dflt"].max_completion_tokens, 8192);
}

#[test]
fn custom_provider_pricing_defaults_to_zero() {
    let tmp = tempdir().expect("tempdir");
    let global = tmp.path().join("providers.json");
    std::fs::write(
        &global,
        r#"{
            "mymodel": {
                "base_url": "http://localhost:8080/v1",
                "default_model": "llama3",
                "env_key": "MY_API_KEY"
            }
        }"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&tmp.path().join("local.json"), &global);
    assert!(loaded.contains_key("mymodel"));
    assert_eq!(loaded["mymodel"].pricing.input, 0.0);
    assert_eq!(loaded["mymodel"].pricing.output, 0.0);
}

#[test]
fn custom_provider_missing_required_field_is_skipped() {
    // `provider add` rejects a record missing base_url/default_model/env_key, so
    // a hand-edited registry entry that omits one is non-functional. The loader
    // must skip it rather than insert a half-formed provider.
    let tmp = tempdir().expect("tempdir");
    let global = tmp.path().join("providers.json");
    std::fs::write(
        &global,
        r#"{
            "no_url":   {"default_model": "m", "env_key": "K"},
            "no_model": {"base_url": "http://x/v1", "env_key": "K"},
            "no_key":   {"base_url": "http://x/v1", "default_model": "m"},
            "blank_url":{"base_url": "", "default_model": "m", "env_key": "K"},
            "good":     {"base_url": "http://x/v1", "default_model": "m", "env_key": "K"}
        }"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&tmp.path().join("local.json"), &global);
    assert_eq!(loaded.len(), 1);
    assert!(loaded.contains_key("good"));
    for skipped in ["no_url", "no_model", "no_key", "blank_url"] {
        assert!(!loaded.contains_key(skipped), "{skipped} should be skipped");
    }
}

#[test]
fn custom_provider_cannot_shadow_builtin() {
    let tmp = tempdir().expect("tempdir");
    let global = tmp.path().join("providers.json");
    std::fs::write(
        &global,
        r#"{
            "claude": {
                "base_url": "http://evil.example.com/v1",
                "default_model": "evil-model",
                "env_key": "EVIL_KEY"
            }
        }"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&tmp.path().join("local.json"), &global);
    assert!(!loaded.contains_key("claude"));
}

#[test]
#[serial]
fn detect_backend_custom_provider_after_builtins() {
    let mut g = EnvGuard::new();
    for key in [
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
    ] {
        g.unset(key);
    }
    g.set("MY_CUSTOM_KEY", "test-key");

    let mut custom: IndexMap<String, CustomProvider> = IndexMap::new();
    custom.insert(
        "myprovider".to_string(),
        CustomProvider {
            name: "myprovider".to_string(),
            base_url: "http://example.com/v1".to_string(),
            default_model: "mymodel".to_string(),
            env_key: "MY_CUSTOM_KEY".to_string(),
            pricing: Pricing {
                input: 0.0,
                output: 0.0,
            },
            temperature: 0.0,
            max_completion_tokens: 8192,
        },
    );

    assert_eq!(detect_backend_with(&custom).as_deref(), Some("myprovider"));
}

#[test]
#[serial]
fn custom_provider_call_llm_routes_via_openai_compat() {
    // A custom provider registered in $HOME/.graphify/providers.json must drive a
    // plain call through the OpenAI-compatible client (#1084). nextest isolates
    // each test in its own process, so the HOME/env edits are safe.
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "choices": [{"message": {"content": "from custom"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 3, "completion_tokens": 4}
            })
            .to_string(),
        )
        .create();

    let home = tempdir().expect("tempdir");
    std::fs::create_dir_all(home.path().join(".graphify")).expect("mkdir .graphify");
    std::fs::write(
        home.path().join(".graphify").join("providers.json"),
        format!(
            r#"{{"custom1": {{"base_url": "{}", "default_model": "m", "env_key": "CUSTOM1_KEY"}}}}"#,
            server.url()
        ),
    )
    .expect("write providers.json");

    let mut g = EnvGuard::new();
    g.set("HOME", &home.path().to_string_lossy());
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("CUSTOM1_KEY", "secret");

    let out = call_llm("ping", "custom1", 32).expect("custom provider call succeeds");
    assert_eq!(out, "from custom");
}

#[test]
#[serial]
fn custom_provider_extract_files_routes_via_openai_compat() {
    // `extract_files_direct` with a custom provider must extract through the
    // OpenAI-compatible client (#1084).
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "choices": [{
                    "message": {"content": "{\"nodes\":[{\"id\":\"n1\"}],\"edges\":[]}"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 3, "completion_tokens": 4}
            })
            .to_string(),
        )
        .create();

    let home = tempdir().expect("tempdir");
    std::fs::create_dir_all(home.path().join(".graphify")).expect("mkdir .graphify");
    std::fs::write(
        home.path().join(".graphify").join("providers.json"),
        format!(
            r#"{{"custom2": {{"base_url": "{}", "default_model": "m", "env_key": "CUSTOM2_KEY"}}}}"#,
            server.url()
        ),
    )
    .expect("write providers.json");

    let work = tempdir().expect("workdir");
    let file = work.path().join("a.py");
    std::fs::write(&file, "def f():\n    return 1\n").expect("write source");

    let mut g = EnvGuard::new();
    g.set("HOME", &home.path().to_string_lossy());
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("CUSTOM2_KEY", "secret");

    let resp = extract_files_direct(&[file], "custom2", None, None, work.path())
        .expect("custom provider extraction succeeds");
    assert_eq!(resp.nodes.len(), 1);
}
