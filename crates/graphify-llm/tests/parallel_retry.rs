//! Coverage tests for `extract_corpus_parallel` and `extract_with_adaptive_retry`
//! driven through a real openai backend with mockito (URL configured via
//! `GRAPHIFY_OPENAI_BASE_URL`).

#![allow(clippy::expect_used, clippy::items_after_statements, unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use graphify_llm::{
    CorpusConfig, extract_corpus_parallel, extract_files_direct, extract_with_adaptive_retry,
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

fn write_files(tmp: &tempfile::TempDir, count: usize) -> Vec<PathBuf> {
    let mut paths = vec![];
    for i in 0..count {
        let p = tmp.path().join(format!("file{i}.py"));
        fs::write(&p, format!("def f{i}():\n    pass\n")).expect("test invariant");
        paths.push(p);
    }
    paths
}

fn good_response_body() -> String {
    json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"n\"}],\"edges\":[]}"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3}
    })
    .to_string()
}

// ── extract_files_direct via mock openai ───────────────────────────────────

#[test]
fn extract_files_direct_via_openai_mock() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(good_response_body())
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 2);
    let resp = extract_files_direct(&files, "openai", Some("k"), Some("m"), tmp.path())
        .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
}

// ── extract_with_adaptive_retry single chunk ───────────────────────────────

#[test]
fn extract_with_adaptive_retry_happy_path() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(good_response_body())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("OPENAI_API_KEY", "k");

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 1);
    let resp =
        extract_with_adaptive_retry(&files, "openai", Some("k"), Some("m"), tmp.path(), 3, 0)
            .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
}

// ── extract_with_adaptive_retry context-overflow split-and-merge ───────────

#[test]
fn extract_with_adaptive_retry_truncated_chunk_bisects() {
    // Returning finish_reason="length" with multiple files in the chunk
    // exercises the truncation bisect path in retry.rs.
    let mut server = mockito::Server::new();
    let trunc_body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"},
            "finish_reason": "length"
        }],
        "usage": {"prompt_tokens": 50, "completion_tokens": 200}
    });
    let _trunc = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(trunc_body.to_string())
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("OPENAI_API_KEY", "k");

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 4);
    let result =
        extract_with_adaptive_retry(&files, "openai", Some("k"), Some("m"), tmp.path(), 3, 0)
            .expect("test invariant");
    // Bisect kept partial result; nodes set non-empty.
    assert!(!result.nodes.is_empty());
}

#[test]
fn extract_with_adaptive_retry_single_file_truncation_keeps_partial() {
    let mut server = mockito::Server::new();
    let trunc_body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"partial\"}],\"edges\":[]}"},
            "finish_reason": "length"
        }],
        "usage": {"prompt_tokens": 50, "completion_tokens": 100}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(trunc_body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("OPENAI_API_KEY", "k");

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 1);
    // Single-file truncation: partial result kept (no further bisect possible).
    let result =
        extract_with_adaptive_retry(&files, "openai", Some("k"), Some("m"), tmp.path(), 3, 0)
            .expect("test invariant");
    assert!(!result.nodes.is_empty());
    assert_eq!(result.finish_reason, "length");
}

#[test]
fn extract_with_adaptive_retry_truncation_at_max_depth() {
    let mut server = mockito::Server::new();
    let trunc_body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}"},
            "finish_reason": "length"
        }],
        "usage": {}
    });
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(trunc_body.to_string())
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("OPENAI_API_KEY", "k");

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 4);
    // Already at max_depth: keep partial result, don't recurse.
    let result =
        extract_with_adaptive_retry(&files, "openai", Some("k"), Some("m"), tmp.path(), 2, 2)
            .expect("test invariant");
    assert!(!result.nodes.is_empty());
}

#[test]
fn extract_with_adaptive_retry_bisects_on_context_overflow() {
    let mut server = mockito::Server::new();
    // Always return success for any chunk size. The first call returns a
    // context-overflow error; subsequent calls succeed.
    // We need to use sequential mocks since mockito doesn't easily express
    // "first call fails, rest succeed". Use two mocks with `expect`.
    let _err = server
        .mock("POST", "/chat/completions")
        .with_status(400)
        .with_body(json!({"error": {"message": "context_length_exceeded"}}).to_string())
        .expect(1)
        .create();
    let _ok = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(good_response_body())
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());
    g.set("OPENAI_API_KEY", "k");

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 4);
    // Should not panic; bisects internally.
    let _ = extract_with_adaptive_retry(&files, "openai", Some("k"), Some("m"), tmp.path(), 3, 0);
}

// ── extract_corpus_parallel ────────────────────────────────────────────────

#[test]
fn extract_corpus_parallel_happy_path() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(good_response_body())
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 4);
    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 2,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
    };
    let (resp, failed) = extract_corpus_parallel(&files, &cfg, None);
    assert!(!resp.nodes.is_empty());
    assert_eq!(failed, 0);
}

#[test]
fn extract_corpus_parallel_with_token_budget() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(good_response_body())
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 3);
    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 8,
        token_budget: Some(60_000),
        max_concurrency: 2,
        max_retry_depth: 1,
    };
    let (resp, _failed) = extract_corpus_parallel(&files, &cfg, None);
    assert!(!resp.nodes.is_empty());
}

#[test]
fn extract_corpus_parallel_with_callback() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(good_response_body())
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 2);
    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 1,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
    };
    let count: Arc<std::sync::Mutex<usize>> = Arc::new(std::sync::Mutex::new(0));
    let count_for_cb = Arc::clone(&count);
    let cb: Box<graphify_llm::ChunkDoneCb> = Box::new(move |_idx, _total, _resp| {
        if let Ok(mut c) = count_for_cb.lock() {
            *c += 1;
        }
    });
    let _ = extract_corpus_parallel(&files, &cfg, Some(cb.as_ref()));
    assert!(*count.lock().expect("mutex") >= 1);
}

#[test]
fn extract_corpus_parallel_empty_files() {
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: std::path::Path::new("."),
        chunk_size: 1,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
    };
    let (resp, failed) = extract_corpus_parallel(&[], &cfg, None);
    assert!(resp.nodes.is_empty());
    assert_eq!(failed, 0);
}
