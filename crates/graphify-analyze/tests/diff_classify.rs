//! Coverage tests for `graph_diff`, `file_category`, `is_concept_node`,
//! `is_json_key_node`.

#![allow(clippy::expect_used)]

use graphify_analyze::{file_category, graph_diff, is_concept_node, is_json_key_node};
use graphify_build::build_from_json;
use serde_json::json;

#[allow(clippy::needless_pass_by_value)] // tests build literals each call
fn graph(nodes: serde_json::Value, edges: serde_json::Value) -> graphify_build::Graph {
    build_from_json(json!({"nodes": nodes, "edges": edges}), false, None).expect("test invariant")
}

// ── graph_diff ──────────────────────────────────────────────────────────────

#[test]
fn graph_diff_no_changes() {
    let g = graph(json!([{"id": "a", "label": "A"}]), json!([]));
    let v = graph_diff(&g, &g);
    assert_eq!(v["summary"], "no changes");
    assert_eq!(v["new_nodes"].as_array().expect("array field").len(), 0);
}

#[test]
fn graph_diff_added_nodes_and_edges() {
    let g_old = graph(json!([{"id": "a", "label": "A"}]), json!([]));
    let g_new = graph(
        json!([{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]),
        json!([{"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"}]),
    );
    let v = graph_diff(&g_old, &g_new);
    let summary = v["summary"].as_str().expect("string field");
    assert!(summary.contains("new node"));
    assert!(summary.contains("new edge"));
}

#[test]
fn graph_diff_removed_nodes() {
    let g_old = graph(
        json!([{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]),
        json!([{"source": "a", "target": "b", "relation": "calls"}]),
    );
    let g_new = graph(json!([{"id": "a", "label": "A"}]), json!([]));
    let v = graph_diff(&g_old, &g_new);
    let s = v["summary"].as_str().expect("string field");
    assert!(s.contains("node removed") || s.contains("edge removed"));
}

#[test]
fn graph_diff_multiple_changes_pluralizes() {
    let g_old = graph(json!([]), json!([]));
    let g_new = graph(
        json!([
            {"id": "a", "label": "A"},
            {"id": "b", "label": "B"},
            {"id": "c", "label": "C"}
        ]),
        json!([
            {"source": "a", "target": "b", "relation": "uses"},
            {"source": "b", "target": "c", "relation": "uses"}
        ]),
    );
    let v = graph_diff(&g_old, &g_new);
    let s = v["summary"].as_str().expect("string field");
    assert!(s.contains("3 new nodes"));
    assert!(s.contains("2 new edges"));
}

// ── file_category ──────────────────────────────────────────────────────────

#[test]
fn file_category_code() {
    assert_eq!(file_category("foo.py"), "code");
    assert_eq!(file_category("foo.rs"), "code");
    assert_eq!(file_category("foo.go"), "code");
    assert_eq!(file_category("foo.js"), "code");
    assert_eq!(file_category("foo.tsx"), "code");
    assert_eq!(file_category("foo.java"), "code");
}

#[test]
fn file_category_paper() {
    assert_eq!(file_category("flash.pdf"), "paper");
}

#[test]
fn file_category_image() {
    assert_eq!(file_category("diagram.png"), "image");
    assert_eq!(file_category("photo.jpg"), "image");
}

#[test]
fn file_category_doc() {
    assert_eq!(file_category("readme.md"), "doc");
    assert_eq!(file_category("notes.txt"), "doc");
}

#[test]
fn file_category_unknown() {
    let cat = file_category("random.xyz");
    // Should be some non-empty fallback string.
    assert_ne!(cat, "");
}

// ── is_concept_node ────────────────────────────────────────────────────────

#[test]
fn is_concept_node_detects_concept_id() {
    let g = graph(
        json!([{"id": "concept_foo", "label": "Concept", "file_type": "concept"}]),
        json!([]),
    );
    assert!(is_concept_node(&g, "concept_foo"));
}

#[test]
fn is_concept_node_returns_false_for_code() {
    let g = graph(
        json!([{"id": "n1", "label": "fn", "file_type": "code", "source_file": "a.py"}]),
        json!([]),
    );
    assert!(!is_concept_node(&g, "n1"));
}

// ── is_json_key_node ──────────────────────────────────────────────────────

#[test]
fn is_json_key_node_for_json_source() {
    let g = graph(
        json!([{"id": "n1", "label": "key", "source_file": "config.json"}]),
        json!([]),
    );
    let _result = is_json_key_node(&g, "n1");
    // Either true or false depending on classification heuristic; we just
    // verify it doesn't panic.
}

#[test]
fn is_json_key_node_for_python_source() {
    let g = graph(
        json!([{"id": "n1", "label": "fn", "source_file": "a.py"}]),
        json!([]),
    );
    assert!(!is_json_key_node(&g, "n1"));
}

#[test]
fn is_concept_or_json_handles_missing_node() {
    let g = graph(json!([{"id": "a", "label": "A"}]), json!([]));
    // Missing node → both classifiers return false (cannot classify without
    // metadata; we deliberately diverge from Python's KeyError for safety).
    assert!(!is_concept_node(&g, "ghost"));
    assert!(!is_json_key_node(&g, "ghost"));
}
