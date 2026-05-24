//! Parity tests against `graphify-py/tests/test_affected_cli.py` and
//! the affected-helper unit tests embedded in
//! `graphify-py/graphify/affected.py`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use graphify_affected::{
    AffectedHit, DEFAULT_AFFECTED_RELATIONS, affected_nodes, format_affected, load_graph,
    resolve_seed,
};
use serde_json::json;
use tempfile::tempdir;

fn write_graph() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    let payload = json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {"id": "target", "label": "Foo", "source_file": "pkg/foo.py", "source_location": "L1"},
            {"id": "caller", "label": "X()", "source_file": "app.py", "source_location": "L4"},
            {"id": "barrel", "label": "__init__.py", "source_file": "pkg/__init__.py"},
            {"id": "consumer", "label": "app.py", "source_file": "app.py"},
        ],
        "links": [
            {"source": "caller", "target": "target", "relation": "calls"},
            {"source": "barrel", "target": "target", "relation": "re_exports"},
            {"source": "consumer", "target": "target", "relation": "imports"},
        ],
    });
    fs::write(&graph_path, payload.to_string()).expect("write");
    (dir, graph_path)
}

#[test]
fn affected_reverse_traverses_impact_edges() {
    let (_dir, path) = write_graph();
    let graph = load_graph(&path).expect("load graph");
    let report = format_affected(&graph, "Foo", DEFAULT_AFFECTED_RELATIONS, 2);
    assert!(
        report.contains("Affected nodes for Foo"),
        "report: {report}"
    );
    assert!(report.contains("X()"), "report: {report}");
    assert!(report.contains("calls"), "report: {report}");
    assert!(report.contains("__init__.py"), "report: {report}");
    assert!(report.contains("re_exports"), "report: {report}");
    assert!(report.contains("app.py"), "report: {report}");
    assert!(report.contains("imports"), "report: {report}");
}

#[test]
fn affected_relation_filter_limits_reverse_traversal() {
    let (_dir, path) = write_graph();
    let graph = load_graph(&path).expect("load graph");
    let report = format_affected(&graph, "Foo", &["calls"], 2);
    assert!(report.contains("Relations: calls"));
    assert!(report.contains("X()"));
    assert!(
        !report.contains("__init__.py"),
        "barrel should be excluded by --relation calls"
    );
}

#[test]
fn resolve_seed_exact_id_match() {
    let (_dir, path) = write_graph();
    let graph = load_graph(&path).expect("load graph");
    assert_eq!(resolve_seed(&graph, "target"), Some("target".to_owned()));
}

#[test]
fn resolve_seed_exact_label_match() {
    let (_dir, path) = write_graph();
    let graph = load_graph(&path).expect("load graph");
    assert_eq!(resolve_seed(&graph, "Foo"), Some("target".to_owned()));
}

#[test]
fn resolve_seed_returns_none_on_ambiguity() {
    // Two nodes share the label "app.py" suffix in their label; "app" is
    // a substring of both "X()"? no — but it is of "app.py" (the consumer
    // node). Build an ambiguous case explicitly.
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true,
        "nodes": [
            {"id": "a", "label": "Foo"},
            {"id": "b", "label": "Foo"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    assert_eq!(resolve_seed(&graph, "Foo"), None);
}

#[test]
fn affected_nodes_respects_depth() {
    // a → target → other; reverse: only nodes one hop back at depth=1.
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true,
        "nodes": [
            {"id": "target", "label": "Foo"},
            {"id": "near", "label": "Near"},
            {"id": "far", "label": "Far"},
        ],
        "links": [
            {"source": "near", "target": "target", "relation": "calls"},
            {"source": "far", "target": "near", "relation": "calls"},
        ],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");

    let hits1 = affected_nodes(&graph, "target", &["calls"], 1);
    assert_eq!(hits1.len(), 1);
    assert_eq!(hits1[0].node_id, "near");
    assert_eq!(hits1[0].depth, 1);

    let hits2 = affected_nodes(&graph, "target", &["calls"], 2);
    assert_eq!(hits2.len(), 2);
    assert!(hits2.iter().any(|h| h.node_id == "near"));
    assert!(hits2.iter().any(|h| h.node_id == "far"));
}

#[test]
fn affected_nodes_skips_other_relations() {
    let (_dir, path) = write_graph();
    let graph = load_graph(&path).expect("load");
    let hits = affected_nodes(&graph, "target", &["imports"], 2);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].node_id, "consumer");
    assert_eq!(hits[0].via_relation, "imports");
}

#[test]
fn affected_format_handles_no_hits() {
    let (_dir, path) = write_graph();
    let graph = load_graph(&path).expect("load");
    let report = format_affected(&graph, "Foo", &["embeds"], 2);
    assert!(report.contains("No affected nodes found."));
}

#[test]
fn affected_format_handles_missing_query() {
    let (_dir, path) = write_graph();
    let graph = load_graph(&path).expect("load");
    let report = format_affected(&graph, "DoesNotExist", DEFAULT_AFFECTED_RELATIONS, 2);
    assert!(report.contains("No unique node match for DoesNotExist"));
}

#[test]
fn affected_hit_struct_carries_expected_fields() {
    let hit = AffectedHit {
        node_id: "x".to_owned(),
        depth: 1,
        via_relation: "calls".to_owned(),
    };
    assert_eq!(hit.node_id, "x");
    assert_eq!(hit.depth, 1);
    assert_eq!(hit.via_relation, "calls");
}
