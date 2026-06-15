//! Parity tests for `graphify_serve::querylog`.
//!
//! Mirrors `graphify-py/tests/test_querylog.py`. Env-var tests are serialized
//! and use a scoped guard; each writes to a `tempdir` path.

#![allow(clippy::expect_used, clippy::unwrap_used, unsafe_code)]

use graphify_serve::querylog::{QueryLog, log_query, nodes_from_result};
use indexmap::IndexMap;
use serde_json::Value;
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

// ── nodes_from_result ──────────────────────────────────────────────────────

#[test]
fn nodes_from_result_parses_header() {
    let result = "Traversal: BFS depth=2 | Start: ['foo'] | 7 nodes found\n\nNODE foo";
    assert_eq!(nodes_from_result(result), Some(7));
}

#[test]
fn nodes_from_result_singular() {
    assert_eq!(nodes_from_result("1 node found"), Some(1));
}

#[test]
fn nodes_from_result_missing() {
    assert_eq!(nodes_from_result("no match here"), None);
}

#[test]
fn nodes_from_result_empty() {
    assert_eq!(nodes_from_result(""), None);
}

// ── log_query basic write ──────────────────────────────────────────────────

fn read_records(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
#[serial(env)]
fn log_query_writes_jsonl() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");
    g.remove("GRAPHIFY_QUERY_LOG_RESPONSES");

    let mut extra = IndexMap::new();
    extra.insert("mode".to_string(), Value::String("bfs".to_string()));
    extra.insert("depth".to_string(), Value::from(2));
    log_query(&QueryLog {
        kind: "query",
        question: "what is X",
        corpus: "/some/graph.json",
        result: Some("3 nodes found\nNODE a"),
        duration_ms: Some(12.5),
        extra,
        ..QueryLog::default()
    });

    let recs = read_records(&log);
    assert_eq!(recs.len(), 1);
    let rec = &recs[0];
    assert_eq!(rec["kind"], "query");
    assert_eq!(rec["question"], "what is X");
    assert_eq!(rec["corpus"], "/some/graph.json");
    assert_eq!(rec["nodes_returned"], 3);
    assert!(rec["result_chars"].as_u64().unwrap() > 0);
    assert!((rec["duration_ms"].as_f64().unwrap() - 12.5).abs() < 0.01);
    assert_eq!(rec["mode"], "bfs");
    assert!(rec.get("ts").is_some());
}

#[test]
#[serial(env)]
fn log_query_appends() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");

    for q in ["q1", "q2"] {
        log_query(&QueryLog {
            kind: "query",
            question: q,
            corpus: "/g.json",
            ..QueryLog::default()
        });
    }
    let recs = read_records(&log);
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0]["question"], "q1");
    assert_eq!(recs[1]["question"], "q2");
}

// ── opt-out / opt-in ───────────────────────────────────────────────────────

#[test]
#[serial(env)]
fn disable_env_disables_logging() {
    for disable in ["1", "true"] {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("q.log");
        let mut g = EnvGuard::new();
        g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
        g.set("GRAPHIFY_QUERY_LOG_DISABLE", disable);

        log_query(&QueryLog {
            kind: "query",
            question: "q",
            corpus: "/g.json",
            ..QueryLog::default()
        });
        assert!(!log.exists(), "log written despite DISABLE={disable}");
    }
}

#[test]
#[serial(env)]
fn responses_not_logged_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");
    g.remove("GRAPHIFY_QUERY_LOG_RESPONSES");

    log_query(&QueryLog {
        kind: "query",
        question: "q",
        corpus: "/g.json",
        result: Some("NODE foo"),
        ..QueryLog::default()
    });
    let rec = &read_records(&log)[0];
    assert!(rec.get("response").is_none());
}

#[test]
#[serial(env)]
fn responses_optin() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.set("GRAPHIFY_QUERY_LOG_RESPONSES", "1");
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");

    log_query(&QueryLog {
        kind: "query",
        question: "q",
        corpus: "/g.json",
        result: Some("NODE foo bar"),
        ..QueryLog::default()
    });
    let rec = &read_records(&log)[0];
    assert_eq!(rec["response"], "NODE foo bar");
}

// ── robustness ─────────────────────────────────────────────────────────────

#[test]
#[serial(env)]
fn log_never_raises_on_bad_path() {
    let tmp = tempfile::tempdir().unwrap();
    // Point at a directory — opening it for append fails.
    let bad = tmp.path().join("is_a_dir");
    std::fs::create_dir(&bad).unwrap();
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &bad.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");

    // Must not panic.
    log_query(&QueryLog {
        kind: "query",
        question: "q",
        corpus: "/g.json",
        ..QueryLog::default()
    });
}

#[test]
#[serial(env)]
fn log_creates_parent_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("deep").join("nested").join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");

    log_query(&QueryLog {
        kind: "query",
        question: "q",
        corpus: "/g.json",
        ..QueryLog::default()
    });
    assert!(log.exists());
}

// ── field coverage ─────────────────────────────────────────────────────────

#[test]
#[serial(env)]
fn nodes_returned_inferred_from_result() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");

    log_query(&QueryLog {
        kind: "query",
        question: "q",
        corpus: "/g.json",
        result: Some("5 nodes found\nNODE a\nNODE b"),
        ..QueryLog::default()
    });
    assert_eq!(read_records(&log)[0]["nodes_returned"], 5);
}

#[test]
#[serial(env)]
fn explicit_nodes_returned_takes_precedence() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");

    log_query(&QueryLog {
        kind: "path",
        question: "A -> B",
        corpus: "/g.json",
        nodes_returned: Some(3),
        ..QueryLog::default()
    });
    assert_eq!(read_records(&log)[0]["nodes_returned"], 3);
}

#[test]
#[serial(env)]
fn kind_mcp_query() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("q.log");
    let mut g = EnvGuard::new();
    g.set("GRAPHIFY_QUERY_LOG", &log.to_string_lossy());
    g.remove("GRAPHIFY_QUERY_LOG_DISABLE");

    log_query(&QueryLog {
        kind: "mcp_query",
        question: "q",
        corpus: "/g.json",
        ..QueryLog::default()
    });
    assert_eq!(read_records(&log)[0]["kind"], "mcp_query");
}
