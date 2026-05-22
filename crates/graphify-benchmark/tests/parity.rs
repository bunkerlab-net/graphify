//! Parity tests against `graphify-py/tests/test_benchmark.py`.
//!
//! Tests for `_safe` / `_hr` encoding-fallback behaviour are Python-specific
//! (Rust stdout is always UTF-8) and are omitted with a note below.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use graphify_benchmark::{
    SAMPLE_QUESTIONS, format_benchmark, hr, query_subgraph_tokens, run_benchmark,
};
use graphify_build::{Graph, GraphKind};
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::io::Write as _;

// ---------------------------------------------------------------------------
// Helper: build the five-node test graph used throughout the Python suite.
// ---------------------------------------------------------------------------

fn make_graph() -> Graph {
    let mut g = Graph::new(GraphKind::Graph);

    let mut add = |id: &str, label: &str, src: &str, loc: &str| {
        let mut attrs = IndexMap::new();
        attrs.insert("label".to_string(), json!(label));
        attrs.insert("source_file".to_string(), json!(src));
        attrs.insert("source_location".to_string(), json!(loc));
        g.add_node(id, attrs);
    };
    add("n1", "authentication", "auth.py", "L1");
    add("n2", "api_handler", "api.py", "L5");
    add("n3", "main_entry", "main.py", "L1");
    add("n4", "error_handler", "errors.py", "L1");
    add("n5", "database_layer", "db.py", "L1");

    let mut edge = |src: &str, tgt: &str, relation: &str, confidence: &str| {
        let mut attrs = IndexMap::new();
        attrs.insert("relation".to_string(), json!(relation));
        attrs.insert("confidence".to_string(), json!(confidence));
        g.add_edge(src, tgt, attrs);
    };
    edge("n1", "n2", "calls", "INFERRED");
    edge("n2", "n3", "imports", "EXTRACTED");
    edge("n3", "n4", "uses", "EXTRACTED");
    edge("n5", "n2", "provides", "EXTRACTED");

    g
}

/// Write a node-link JSON file that `load_graph` can read.
///
/// We produce the minimal format: `{"nodes": [...], "links": [...]}` which
/// mirrors what `networkx.readwrite.json_graph.node_link_data(G, edges="links")`
/// would emit for a simple graph.
fn write_graph(g: &Graph, path: &std::path::Path) {
    let nodes: Vec<Value> = g
        .nodes()
        .map(|(id, attrs)| {
            let mut obj = serde_json::Map::new();
            obj.insert("id".to_string(), json!(id));
            for (k, v) in attrs {
                obj.insert(k.clone(), v.clone());
            }
            Value::Object(obj)
        })
        .collect();

    let links: Vec<Value> = g
        .edges()
        .map(|e| {
            let mut obj = serde_json::Map::new();
            obj.insert("source".to_string(), json!(e.source));
            obj.insert("target".to_string(), json!(e.target));
            for (k, v) in &e.attrs {
                obj.insert(k.clone(), v.clone());
            }
            Value::Object(obj)
        })
        .collect();

    let data = json!({ "nodes": nodes, "links": links });
    let mut f = std::fs::File::create(path).expect("create");
    f.write_all(serde_json::to_string(&data).expect("serialize").as_bytes())
        .expect("write");
}

// ---------------------------------------------------------------------------
// query_subgraph_tokens
// ---------------------------------------------------------------------------

#[test]
fn test_query_returns_positive_for_matching_question() {
    let g = make_graph();
    let tokens = query_subgraph_tokens(&g, "how does authentication work", 3);
    assert!(tokens > 0);
}

#[test]
fn test_query_returns_zero_for_no_match() {
    let g = make_graph();
    let tokens = query_subgraph_tokens(&g, "xyzzy plugh zorkmid", 3);
    assert_eq!(tokens, 0);
}

#[test]
fn test_query_bfs_expands_neighbors() {
    let g = make_graph();
    // "authentication" matches n1; BFS depth=3 should reach more nodes than depth=1.
    let tokens_deep = query_subgraph_tokens(&g, "authentication", 3);
    let tokens_shallow = query_subgraph_tokens(&g, "authentication", 1);
    assert!(tokens_deep >= tokens_shallow);
}

// ---------------------------------------------------------------------------
// run_benchmark
// ---------------------------------------------------------------------------

#[test]
fn test_run_benchmark_returns_reduction() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_file = tmp.path().join("graph.json");
    write_graph(&make_graph(), &graph_file);

    let result = run_benchmark(&graph_file, Some(10_000), None)
        .expect("run_benchmark")
        .expect("some result");
    assert!(result.reduction_ratio > 1.0);
}

#[test]
fn test_run_benchmark_corpus_tokens_proportional() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_file = tmp.path().join("graph.json");
    write_graph(&make_graph(), &graph_file);

    let r1 = run_benchmark(&graph_file, Some(1_000), None)
        .expect("run_benchmark")
        .expect("some result");
    let r2 = run_benchmark(&graph_file, Some(10_000), None)
        .expect("run_benchmark")
        .expect("some result");

    // corpus_tokens scales linearly with corpus_words (within integer-division rounding).
    let scaled = r1.corpus_tokens * 10;
    let diff = r2.corpus_tokens.abs_diff(scaled);
    assert!(diff <= r1.corpus_tokens);
}

#[test]
fn test_run_benchmark_per_question_list() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_file = tmp.path().join("graph.json");
    write_graph(&make_graph(), &graph_file);

    let questions = &["how does authentication work", "what is the main entry"][..];
    let result = run_benchmark(&graph_file, Some(5_000), Some(questions))
        .expect("run_benchmark")
        .expect("some result");

    assert!(!result.per_question.is_empty());
    for p in &result.per_question {
        assert!(!p.question.is_empty());
        assert!(p.query_tokens > 0);
        assert!(p.reduction > 0.0);
    }
}

#[test]
fn test_run_benchmark_estimates_corpus_if_no_words() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_file = tmp.path().join("graph.json");
    write_graph(&make_graph(), &graph_file);

    let result = run_benchmark(&graph_file, None, None)
        .expect("run_benchmark")
        .expect("some result");
    assert!(result.corpus_words > 0);
}

#[test]
fn test_run_benchmark_error_on_empty_graph() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_file = tmp.path().join("empty.json");
    let empty_graph = Graph::new(GraphKind::Graph);
    write_graph(&empty_graph, &graph_file);

    let result = run_benchmark(&graph_file, Some(1_000), None).expect("run_benchmark");
    // No nodes → no matches → None (Python returns {"error": ...}).
    assert!(result.is_none());
}

#[test]
fn test_run_benchmark_includes_node_edge_counts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_file = tmp.path().join("graph.json");
    let g = make_graph();
    let expected_nodes = g.node_count();
    let expected_edges = g.edge_count();
    write_graph(&g, &graph_file);

    let result = run_benchmark(&graph_file, Some(5_000), None)
        .expect("run_benchmark")
        .expect("some result");
    assert_eq!(result.nodes, expected_nodes);
    assert_eq!(result.edges, expected_edges);
}

// ---------------------------------------------------------------------------
// format_benchmark / print_benchmark
// ---------------------------------------------------------------------------

#[test]
fn test_format_benchmark_no_crash() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_file = tmp.path().join("graph.json");
    write_graph(&make_graph(), &graph_file);

    let result = run_benchmark(&graph_file, Some(5_000), None)
        .expect("run_benchmark")
        .expect("some result");
    let out = format_benchmark(Some(&result));
    let lower = out.to_lowercase();
    assert!(lower.contains("reduction"));
    assert!(out.contains('x'));
}

#[test]
fn test_format_benchmark_error_message() {
    let out = format_benchmark(None);
    assert!(out.contains("No matching nodes found"));
}

// ---------------------------------------------------------------------------
// hr() — Rust always uses UTF-8 so we only test the happy path.
// (Python's _safe / encoding-fallback tests are irrelevant in Rust.)
// ---------------------------------------------------------------------------

#[test]
fn test_hr_returns_box_drawing_chars() {
    let rule = hr(5);
    assert_eq!(rule, "─".repeat(5));
}

#[test]
fn test_hr_length() {
    for n in [0, 1, 10, 50] {
        assert_eq!(hr(n).chars().count(), n);
    }
}

// ---------------------------------------------------------------------------
// SAMPLE_QUESTIONS constant is exported.
// ---------------------------------------------------------------------------

#[test]
fn test_sample_questions_not_empty() {
    assert!(!SAMPLE_QUESTIONS.is_empty());
}
