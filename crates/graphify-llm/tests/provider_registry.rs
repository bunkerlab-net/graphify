//! Parity tests for the custom LLM provider registry (#1084).
//!
//! Mirrors `graphify-py/tests/test_provider_registry.py`.
#![allow(clippy::expect_used, clippy::float_cmp)]

use graphify_llm::{
    CustomProvider, Pricing, call_llm, detect_backend_with, extract_files_direct,
    load_custom_providers_from, provider_base_url_ok,
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
fn custom_provider_extra_body_parsed_and_defaulted() {
    // A provider may set `extra_body` (forwarded verbatim to the OpenAI-compat
    // request, #7477b46); when omitted or explicitly null it stays `None`.
    let tmp = tempdir().expect("tempdir");
    let global = tmp.path().join("providers.json");
    std::fs::write(
        &global,
        r#"{
            "vllm": {"base_url": "http://x/v1", "default_model": "m", "env_key": "K",
                     "extra_body": {"chat_template_kwargs": {"enable_thinking": false}}},
            "nulled": {"base_url": "http://y/v1", "default_model": "m", "env_key": "K", "extra_body": null},
            "plain": {"base_url": "http://z/v1", "default_model": "m", "env_key": "K"}
        }"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&tmp.path().join("local.json"), &global);
    assert_eq!(
        loaded["vllm"].extra_body,
        Some(serde_json::json!({"chat_template_kwargs": {"enable_thinking": false}}))
    );
    // A JSON null is treated as absent (matches Python's `cfg.get`).
    assert_eq!(loaded["nulled"].extra_body, None);
    assert_eq!(loaded["plain"].extra_body, None);
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

// ── F1: project-local gating + base_url validation (port of test_provider_registry.py) ──

#[test]
#[serial]
fn project_local_providers_ignored_without_optin() {
    // A project-local ./.graphify/providers.json is NOT loaded by default (F1):
    // it travels with a cloned/shared repo and controls where the corpus + API
    // key are sent, so loading it silently is an exfiltration vector.
    let tmp = tempdir().expect("tempdir");
    let local = tmp.path().join("local.json");
    std::fs::write(
        &local,
        r#"{"evil": {"base_url": "https://attacker.example/v1", "default_model": "m", "env_key": "K"}}"#,
    )
    .expect("write local.json");
    let missing_global = tmp.path().join("global.json"); // does not exist

    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_ALLOW_LOCAL_PROVIDERS");

    let loaded = load_custom_providers_from(&local, &missing_global);
    assert!(!loaded.contains_key("evil"));
}

#[test]
#[serial]
fn project_local_providers_loaded_with_optin() {
    // With explicit opt-in the project-local file is honoured (F1).
    let tmp = tempdir().expect("tempdir");
    let local = tmp.path().join("local.json");
    std::fs::write(
        &local,
        r#"{"lab": {"base_url": "https://lab.internal/v1", "default_model": "m", "env_key": "K"}}"#,
    )
    .expect("write local.json");
    let missing_global = tmp.path().join("global.json");

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_ALLOW_LOCAL_PROVIDERS", "1");

    let loaded = load_custom_providers_from(&local, &missing_global);
    assert!(loaded.contains_key("lab"));
}

#[test]
fn non_http_provider_base_url_rejected() {
    // A provider whose base_url uses a non-http(s) scheme is skipped on load (F1).
    let tmp = tempdir().expect("tempdir");
    let global = tmp.path().join("providers.json");
    std::fs::write(
        &global,
        r#"{"sneaky": {"base_url": "file:///etc/passwd", "default_model": "m", "env_key": "K"}}"#,
    )
    .expect("write providers.json");

    let loaded = load_custom_providers_from(&tmp.path().join("local.json"), &global);
    assert!(!loaded.contains_key("sneaky"));
}

#[test]
fn provider_base_url_ok_scheme_and_warnings() {
    // Rejects bad schemes; allows http(s); plaintext-http egress warns but loads.
    // The third argument is `warn = true`, so this intentionally exercises the
    // stderr-warning code paths.
    assert!(provider_base_url_ok("https://api.example/v1", "ok", true));
    assert!(provider_base_url_ok(
        "http://localhost:11434/v1",
        "local",
        true
    ));
    assert!(!provider_base_url_ok("file:///etc/passwd", "bad", true));
    assert!(!provider_base_url_ok("gopher://x/", "bad2", true));
    // Plaintext http to a non-loopback host loads (warns to stderr).
    assert!(provider_base_url_ok("http://example.com/v1", "plain", true));
    // A `127.`-prefixed *hostname* (not a 127.0.0.0/8 IP) is non-loopback, so it
    // still loads but is treated as plaintext egress rather than silenced.
    assert!(provider_base_url_ok(
        "http://127.evil.com/v1",
        "tricky",
        true
    ));
    // Bracketed IPv6 loopback is silenced; IPv6 link-local loads but warns —
    // both exercise the bracket-stripping + `IpAddr::is_loopback` path.
    assert!(provider_base_url_ok(
        "http://[::1]:8080/v1",
        "v6local",
        true
    ));
    assert!(provider_base_url_ok("http://[fe80::1]/v1", "v6ll", true));
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
    g.scrub_backends();
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
            extra_body: None,
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
    // Hermetic setup: clear built-in backend keys so this stays a pure
    // custom-provider routing test even if `call_llm` ever grows a built-in
    // fallback. (`CUSTOM1_KEY` is a registry env_key, untouched by scrub.)
    g.scrub_backends();
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
    // Hermetic setup: see `custom_provider_call_llm_routes_via_openai_compat`.
    g.scrub_backends();
    g.set("HOME", &home.path().to_string_lossy());
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("CUSTOM2_KEY", "secret");

    let resp = extract_files_direct(&[file], "custom2", None, None, work.path())
        .expect("custom provider extraction succeeds");
    assert_eq!(resp.nodes.len(), 1);
}
