//! Custom-endpoint env overrides for the openai/claude backends (#61836ce) and
//! the openai-compat output-token cap (#5b0c154).
//!
//! Mirrors graphify-py `tests/test_openai_custom_endpoint.py` and
//! `tests/test_anthropic_custom_endpoint.py`. Python reloads the module to
//! re-read import-time env; the Rust ports resolve at call time, so each test
//! scrubs the relevant vars under `#[serial(env)]`.
#![allow(clippy::expect_used, unsafe_code)]

use graphify_llm::{backend_config, claude, deepseek, gemini, kimi, openai};
use serial_test::serial;

mod common;
use common::EnvGuard;

// ── openai base_url / default_model ─────────────────────────────────────────

#[test]
#[serial(env)]
fn openai_defaults_without_env() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_OPENAI_BASE_URL")
        .unset("OPENAI_BASE_URL")
        .unset("GRAPHIFY_OPENAI_MODEL")
        .unset("OPENAI_MODEL");
    assert_eq!(openai::base_url(), "https://api.openai.com/v1");
    assert_eq!(openai::default_model().as_ref(), "gpt-4.1-mini");
}

#[test]
#[serial(env)]
fn openai_base_url_and_model_env_override() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_OPENAI_BASE_URL")
        .unset("GRAPHIFY_OPENAI_MODEL")
        .set("OPENAI_BASE_URL", "http://localhost:8080/v1")
        .set("OPENAI_MODEL", "my-local-model");
    assert_eq!(openai::base_url(), "http://localhost:8080/v1");
    assert_eq!(openai::default_model().as_ref(), "my-local-model");
}

#[test]
#[serial(env)]
fn graphify_openai_model_wins_over_openai_model() {
    // model_env_key (GRAPHIFY_OPENAI_MODEL) takes precedence over OPENAI_MODEL.
    let mut g = EnvGuard::new();
    g.set("OPENAI_MODEL", "env-default-model")
        .set("GRAPHIFY_OPENAI_MODEL", "graphify-override-model");
    assert_eq!(openai::default_model().as_ref(), "graphify-override-model");
}

#[test]
#[serial(env)]
fn graphify_openai_base_url_wins_over_openai_base_url() {
    // GRAPHIFY_OPENAI_BASE_URL (the test-redirect var) wins over OPENAI_BASE_URL.
    let mut g = EnvGuard::new();
    g.set("OPENAI_BASE_URL", "http://upstream:8080/v1")
        .set("GRAPHIFY_OPENAI_BASE_URL", "http://redirect:9090/v1");
    assert_eq!(openai::base_url(), "http://redirect:9090/v1");
}

// ── claude base_url / default_model ─────────────────────────────────────────

#[test]
#[serial(env)]
fn claude_defaults_without_env() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_CLAUDE_BASE_URL")
        .unset("ANTHROPIC_BASE_URL")
        .unset("GRAPHIFY_CLAUDE_MODEL")
        .unset("ANTHROPIC_MODEL");
    assert_eq!(claude::base_url(), "https://api.anthropic.com");
    assert_eq!(claude::default_model().as_ref(), "claude-sonnet-4-6");
}

#[test]
#[serial(env)]
fn claude_base_url_and_model_env_override() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_CLAUDE_BASE_URL")
        .unset("GRAPHIFY_CLAUDE_MODEL")
        .set("ANTHROPIC_BASE_URL", "http://localhost:4000")
        .set("ANTHROPIC_MODEL", "my-proxied-model");
    assert_eq!(claude::base_url(), "http://localhost:4000");
    assert_eq!(claude::default_model().as_ref(), "my-proxied-model");
}

#[test]
#[serial(env)]
fn graphify_claude_model_wins_over_anthropic_model() {
    let mut g = EnvGuard::new();
    g.set("ANTHROPIC_MODEL", "env-default-model")
        .set("GRAPHIFY_CLAUDE_MODEL", "graphify-override-model");
    assert_eq!(claude::default_model().as_ref(), "graphify-override-model");
}

// ── output-token cap (#5b0c154): openai-compat backends resolve 16384 ───────

#[test]
#[serial(env)]
fn openai_compat_backends_resolve_full_output_cap() {
    // #1365: these configs define max_tokens 16384; the dispatch reads the
    // unified `default_max_tokens` field, so every openai-compat backend resolves
    // 16384 (not the old 8192 fallback). GRAPHIFY_MAX_OUTPUT_TOKENS unset.
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_MAX_OUTPUT_TOKENS");
    for backend in ["openai", "ollama", "deepseek", "kimi"] {
        let cfg = backend_config(backend).expect("known backend");
        assert_eq!(
            cfg.default_max_tokens, 16_384,
            "{backend} should cap output at 16384"
        );
        assert_eq!(
            graphify_llm::openai_compat::resolve_max_tokens(cfg.default_max_tokens),
            16_384,
            "{backend} resolved cap should be 16384"
        );
    }
    // The openai backend's own default-max-tokens helper agrees.
    assert_eq!(openai::default_max_tokens(), 16_384);
}

// ── kimi / gemini / deepseek bare *_BASE_URL env overrides (#1458) ────────────

#[test]
#[serial(env)]
fn kimi_base_url_honors_bare_env() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_KIMI_BASE_URL")
        .set("KIMI_BASE_URL", "https://proxy.example/kimi/v1");
    assert_eq!(kimi::base_url(), "https://proxy.example/kimi/v1");
}

#[test]
#[serial(env)]
fn gemini_base_url_honors_bare_env() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_GEMINI_BASE_URL")
        .set("GEMINI_BASE_URL", "https://proxy.example/gemini");
    assert_eq!(gemini::base_url(), "https://proxy.example/gemini");
}

#[test]
#[serial(env)]
fn deepseek_base_url_honors_bare_env() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_DEEPSEEK_BASE_URL")
        .set("DEEPSEEK_BASE_URL", "https://proxy.example/deepseek");
    assert_eq!(deepseek::base_url(), "https://proxy.example/deepseek");
}

#[test]
#[serial(env)]
fn kimi_gemini_deepseek_defaults_without_env() {
    let mut g = EnvGuard::new();
    g.unset("GRAPHIFY_KIMI_BASE_URL")
        .unset("KIMI_BASE_URL")
        .unset("GRAPHIFY_GEMINI_BASE_URL")
        .unset("GEMINI_BASE_URL")
        .unset("GRAPHIFY_DEEPSEEK_BASE_URL")
        .unset("DEEPSEEK_BASE_URL");
    assert_eq!(kimi::base_url(), "https://api.moonshot.ai/v1");
    assert_eq!(
        gemini::base_url(),
        "https://generativelanguage.googleapis.com/v1beta/openai/"
    );
    assert_eq!(deepseek::base_url(), "https://api.deepseek.com");
}

#[test]
#[serial(env)]
fn graphify_kimi_base_url_wins_over_bare() {
    // The GRAPHIFY_-prefixed test-redirect var takes priority over the bare one,
    // mirroring the openai precedence.
    let mut g = EnvGuard::new();
    g.set("KIMI_BASE_URL", "https://upstream/kimi/v1")
        .set("GRAPHIFY_KIMI_BASE_URL", "https://redirect/kimi/v1");
    assert_eq!(kimi::base_url(), "https://redirect/kimi/v1");
}
