//! Coverage tests for `extract_corpus_parallel` and `extract_with_adaptive_retry`
//! driven through a real openai backend with mockito (URL configured via
//! `GRAPHIFY_OPENAI_BASE_URL`).

#![allow(clippy::expect_used, clippy::items_after_statements, unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use graphify_llm::{
    CorpusConfig, extract_corpus_parallel, extract_files_direct, extract_files_direct_mode,
    extract_with_adaptive_retry,
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

#[test]
fn extract_files_direct_deep_mode_sends_deep_prompt() {
    // The mock only matches when the request body carries the deep-mode system
    // prompt, so a passing request proves `--mode deep` reached the wire.
    let mut server = mockito::Server::new();
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::Regex("DEEP_MODE".to_string()))
        .with_status(200)
        .with_body(good_response_body())
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 1);
    let resp = extract_files_direct_mode(&files, "openai", Some("k"), Some("m"), tmp.path(), true)
        .expect("test invariant");
    assert_eq!(resp.nodes.len(), 1);
    m.assert();
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
        deep_mode: false,
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
        deep_mode: false,
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
        deep_mode: false,
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
        deep_mode: false,
    };
    let (resp, failed) = extract_corpus_parallel(&[], &cfg, None);
    assert!(resp.nodes.is_empty());
    assert_eq!(failed, 0);
}

#[test]
fn omitted_documents_are_reconciled_and_warned() {
    // #1890: a single chunk returns a clean response that mentions only file0
    // and file2, silently omitting file1 and file3. Reconciliation must surface
    // the omitted files in `uncovered_files` (sorted), not drop them.
    let mut server = mockito::Server::new();
    let body = json!({
        "choices": [{
            "message": {"content": "{\"nodes\":[{\"id\":\"n0\",\"source_file\":\"file0.py\"},{\"id\":\"n2\",\"source_file\":\"file2.py\"}],\"edges\":[]}"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3}
    })
    .to_string();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body)
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let files = write_files(&tmp, 4);
    // One chunk holds all four files so the omission is within a single response.
    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 8,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
        deep_mode: false,
    };
    let (resp, _failed) = extract_corpus_parallel(&files, &cfg, None);
    let mut names: Vec<String> = resp
        .uncovered_files
        .iter()
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["file1.py".to_string(), "file3.py".to_string()]);
}
// ── #1632: parallel merge is deterministic (submission order, not completion) ──

#[test]
fn extract_corpus_parallel_merge_order_is_deterministic() {
    // Each file gets a distinct node id via a body-matched mock; with 4 chunks
    // running concurrently, completion order is nondeterministic. The merged
    // node order must nonetheless be stable run-to-run (submission order), so
    // graph.json is byte-identical across runs (#1632).
    let mut server = mockito::Server::new();
    for i in 0..4 {
        let body = json!({
            "choices": [{
                "message": {"content": format!("{{\"nodes\":[{{\"id\":\"node_from_f{i}\"}}],\"edges\":[]}}")},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 2, "completion_tokens": 3}
        })
        .to_string();
        server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex(format!("def f{i}")))
            .with_status(200)
            .with_body(body)
            .expect_at_least(1)
            .create();
    }

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
        chunk_size: 1,
        token_budget: None,
        max_concurrency: 4,
        max_retry_depth: 1,
        deep_mode: false,
    };

    let node_ids = |resp: &graphify_llm::LlmResponse| -> Vec<String> {
        resp.nodes
            .iter()
            .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect()
    };

    let (resp1, _) = extract_corpus_parallel(&files, &cfg, None);
    let (resp2, _) = extract_corpus_parallel(&files, &cfg, None);
    let order1 = node_ids(&resp1);
    let order2 = node_ids(&resp2);
    assert_eq!(order1.len(), 4, "all four chunks should merge: {order1:?}");
    assert_eq!(
        order1, order2,
        "merge order must be identical across runs (deterministic)"
    );
}

// ── #1895: out-of-scope nodes are dropped from the merged result ──────────────

#[test]
fn out_of_scope_nodes_are_dropped_from_merged_result() {
    // #1895: the #1757 cache guard skips the CACHE write for a node attributed to
    // a real corpus file that was not dispatched, but the node itself still
    // reached the merged result. Drop such nodes (and edges/hyperedges touching
    // them), record the count, and keep in-scope sibling + non-file concept
    // attributions.
    let mut server = mockito::Server::new();
    let inner = json!({
        "nodes": [
            {"id": "a_ok", "source_file": "A.md", "file_type": "document"},
            {"id": "c_sibling", "source_file": "C.md", "file_type": "document"},
            {"id": "b_stray", "source_file": "B.py", "file_type": "code"},
            {"id": "auth_flow", "source_file": "auth flow", "file_type": "concept"},
        ],
        "edges": [
            {"source": "a_ok", "target": "c_sibling", "source_file": "A.md"},
            {"source": "a_ok", "target": "b_stray", "source_file": "A.md"},
        ],
        "hyperedges": [
            {"id": "h_bad", "nodes": ["a_ok", "c_sibling", "b_stray"], "source_file": "A.md"},
            {"id": "h_ok", "nodes": ["a_ok", "c_sibling", "auth_flow"], "source_file": "A.md"},
        ],
    })
    .to_string();
    let body = json!({
        "choices": [{"message": {"content": inner}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3}
    })
    .to_string();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body)
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("A.md");
    fs::write(&a, "# a\n").expect("write");
    let c = tmp.path().join("C.md");
    fs::write(&c, "# c\n").expect("write");
    // B.py exists on disk but is NOT dispatched — the #1895 out-of-scope case.
    fs::write(tmp.path().join("B.py"), "def b():\n    pass\n").expect("write");

    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 8,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
        deep_mode: false,
    };
    let (resp, _failed) = extract_corpus_parallel(&[a, c], &cfg, None);

    let ids: std::collections::HashSet<&str> = resp
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert!(
        !ids.contains("b_stray"),
        "out-of-scope node leaked: {ids:?}"
    );
    for id in ["a_ok", "c_sibling", "auth_flow"] {
        assert!(ids.contains(id), "in-scope attribution dropped: {id}");
    }
    assert_eq!(resp.out_of_scope_dropped, 1);
    assert!(
        resp.edges.iter().any(|e| {
            e.get("source").and_then(serde_json::Value::as_str) == Some("a_ok")
                && e.get("target").and_then(serde_json::Value::as_str) == Some("c_sibling")
        }),
        "in-scope edge a_ok -> c_sibling was wrongly dropped: {:?}",
        resp.edges
    );
    assert!(
        resp.edges.iter().all(|e| {
            let src = e.get("source").and_then(serde_json::Value::as_str);
            let tgt = e.get("target").and_then(serde_json::Value::as_str);
            src != Some("b_stray") && tgt != Some("b_stray")
        }),
        "edge to dropped node survived: {:?}",
        resp.edges
    );
    let he_ids: Vec<&str> = resp
        .hyperedges
        .iter()
        .filter_map(|h| h.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(he_ids, ["h_ok"], "hyperedge touching dropped node kept");
    assert!(
        resp.uncovered_files.is_empty(),
        "dispatched files all produced nodes"
    );
}

#[test]
fn out_of_scope_edges_dropped_even_when_all_nodes_in_scope() {
    // #1895 divergence from graphify-py (`llm.py:2041`): a relationship attributed
    // to a real, undispatched file must be dropped even when NO node is
    // out-of-scope. Both nodes here belong to the dispatched A.md; the B.py edge
    // and hyperedge are out-of-scope and must not survive, while the A.md ones do.
    let mut server = mockito::Server::new();
    let inner = json!({
        "nodes": [
            {"id": "a_ok", "source_file": "A.md", "file_type": "document"},
            {"id": "a2_ok", "source_file": "A.md", "file_type": "document"},
        ],
        "edges": [
            {"source": "a_ok", "target": "a2_ok", "source_file": "A.md"},
            {"source": "a_ok", "target": "a2_ok", "source_file": "B.py"},
        ],
        "hyperedges": [
            {"id": "h_ok", "nodes": ["a_ok", "a2_ok"], "source_file": "A.md"},
            {"id": "h_stray", "nodes": ["a_ok", "a2_ok"], "source_file": "B.py"},
        ],
    })
    .to_string();
    let body = json!({
        "choices": [{"message": {"content": inner}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3}
    })
    .to_string();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body)
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("A.md");
    fs::write(&a, "# a\n").expect("write");
    // B.py exists on disk but is NOT dispatched.
    fs::write(tmp.path().join("B.py"), "def b():\n    pass\n").expect("write");

    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 8,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
        deep_mode: false,
    };
    let (resp, _failed) = extract_corpus_parallel(&[a], &cfg, None);

    // No node is out-of-scope, so the node-drop count stays zero ...
    assert_eq!(resp.out_of_scope_dropped, 0);
    let ids: std::collections::HashSet<&str> = resp
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert!(ids.contains("a_ok") && ids.contains("a2_ok"));
    // ... but the out-of-scope relationships are still filtered.
    let edge_files: Vec<&str> = resp
        .edges
        .iter()
        .filter_map(|e| e.get("source_file").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(edge_files, ["A.md"], "out-of-scope B.py edge survived");
    let he_ids: Vec<&str> = resp
        .hyperedges
        .iter()
        .filter_map(|h| h.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(he_ids, ["h_ok"], "out-of-scope B.py hyperedge survived");
}

#[test]
fn out_of_scope_keeps_edge_to_id_also_on_in_scope_node() {
    // #1895 duplicate attribution: the model emits the same id under both an
    // in-scope file (A.md) and an undispatched one (B.py). The out-of-scope copy
    // is dropped, but the id survives via the in-scope node, so an edge to it
    // must NOT be pruned.
    let mut server = mockito::Server::new();
    let inner = json!({
        "nodes": [
            {"id": "a_ok", "source_file": "A.md", "file_type": "document"},
            {"id": "shared", "source_file": "A.md", "file_type": "document"},
            {"id": "shared", "source_file": "B.py", "file_type": "code"},
        ],
        "edges": [
            {"source": "a_ok", "target": "shared", "source_file": "A.md"},
        ],
    })
    .to_string();
    let body = json!({
        "choices": [{"message": {"content": inner}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3}
    })
    .to_string();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body)
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("A.md");
    fs::write(&a, "# a\n").expect("write");
    fs::write(tmp.path().join("B.py"), "def b():\n    pass\n").expect("write");

    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 8,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
        deep_mode: false,
    };
    let (resp, _failed) = extract_corpus_parallel(&[a], &cfg, None);

    // The out-of-scope B.py copy is counted dropped, but "shared" survives.
    assert_eq!(resp.out_of_scope_dropped, 1);
    let ids: Vec<&str> = resp
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert!(ids.contains(&"a_ok"), "a_ok must survive: {ids:?}");
    let shared: Vec<&serde_json::Value> = resp
        .nodes
        .iter()
        .filter(|n| n.get("id").and_then(serde_json::Value::as_str) == Some("shared"))
        .collect();
    assert_eq!(
        shared.len(),
        1,
        "exactly one `shared` node must survive: {ids:?}"
    );
    assert_eq!(
        shared[0]
            .get("source_file")
            .and_then(serde_json::Value::as_str),
        Some("A.md"),
        "the surviving `shared` node must be the in-scope A.md copy"
    );
    // The edge to the (surviving) shared id must be kept.
    assert!(
        resp.edges.iter().any(|e| {
            e.get("source").and_then(serde_json::Value::as_str) == Some("a_ok")
                && e.get("target").and_then(serde_json::Value::as_str) == Some("shared")
        }),
        "edge to a shared id that survives in scope was wrongly pruned: {:?}",
        resp.edges
    );
}

#[test]
fn out_of_scope_drop_count_is_zero_when_all_in_scope() {
    // Counter-test: a clean run records out_of_scope_dropped == 0.
    let mut server = mockito::Server::new();
    let inner = json!({
        "nodes": [{"id": "a_ok", "source_file": "A.md", "file_type": "document"}],
        "edges": [],
        "hyperedges": [],
    })
    .to_string();
    let body = json!({
        "choices": [{"message": {"content": inner}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3}
    })
    .to_string();
    let _m = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_body(body)
        .expect_at_least(1)
        .create();

    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS", "1");
    g.set("GRAPHIFY_OPENAI_BASE_URL", &server.url());

    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("A.md");
    fs::write(&a, "# a\n").expect("write");
    let cfg = CorpusConfig {
        backend: "openai",
        api_key: Some("k"),
        model: Some("m"),
        root: tmp.path(),
        chunk_size: 1,
        token_budget: None,
        max_concurrency: 1,
        max_retry_depth: 1,
        deep_mode: false,
    };
    let (resp, _failed) = extract_corpus_parallel(&[a], &cfg, None);
    assert_eq!(resp.out_of_scope_dropped, 0);
    let ids: Vec<&str> = resp
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(ids, ["a_ok"]);
    // out_of_scope_dropped is observability-only: it must NOT leak into the
    // serialized graph JSON the build path consumes.
    assert!(
        resp.to_value().get("out_of_scope_dropped").is_none(),
        "out_of_scope_dropped must not be persisted to graph.json"
    );
}
