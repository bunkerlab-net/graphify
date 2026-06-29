//! Parity tests against `graphify-py/tests/test_affected_cli.py` and
//! the affected-helper unit tests embedded in
//! `graphify-py/graphify/affected.py`.
#![allow(clippy::expect_used)]

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

// Mirrors: test_affected_cli_forces_directed_on_undirected_graph
#[test]
fn affected_forces_directed_on_undirected_graph() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("graph.json");
    let payload = json!({
        "directed": false,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {"id": "A", "label": "caller_fn", "source_file": "a.py", "source_location": "L1"},
            {"id": "B", "label": "callee_fn", "source_file": "b.py", "source_location": "L2"},
        ],
        "links": [
            {"source": "A", "target": "B", "relation": "calls",
             "context": "call", "confidence": "EXTRACTED"},
        ],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    // A (the caller) is affected by a change to B (the callee).
    let report = format_affected(&graph, "B", &["calls"], 2);
    assert!(report.contains("caller_fn"), "report: {report}");
    assert!(report.contains("calls"), "report: {report}");
    assert!(
        !report.contains("No affected nodes found."),
        "report: {report}"
    );
}

// Mirrors: test_affected_cli_loads_edges_keyed_graph
#[test]
fn affected_loads_edges_keyed_graph() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("graph.json");
    let payload = json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {"id": "target", "label": "Foo", "source_file": "pkg/foo.py", "source_location": "L1"},
            {"id": "caller", "label": "X()", "source_file": "app.py", "source_location": "L4"},
        ],
        // graphify `extract` output uses an "edges" key, not networkx's "links".
        "edges": [
            {"source": "caller", "target": "target", "relation": "calls",
             "context": "call", "confidence": "EXTRACTED"},
        ],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    let report = format_affected(&graph, "Foo", DEFAULT_AFFECTED_RELATIONS, 2);
    assert!(
        report.contains("Affected nodes for Foo"),
        "report: {report}"
    );
    assert!(report.contains("X()"), "report: {report}");
    assert!(report.contains("calls"), "report: {report}");
}

// Mirrors: test_resolve_seed_bare_name_matches_callable_label
#[test]
fn resolve_seed_bare_name_matches_callable_label() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true,
        "nodes": [
            {"id": "a", "label": "classifyProperty()", "source_file": "pkg/entity.py"},
            {"id": "b", "label": "classifyPropertySafe()", "source_file": "app/context.py"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    assert_eq!(
        resolve_seed(&graph, "classifyProperty"),
        Some("a".to_owned())
    );
    assert_eq!(
        resolve_seed(&graph, "classifyPropertySafe"),
        Some("b".to_owned())
    );
}

// Mirrors: test_resolve_seed_decorated_query_matches_bare_label
#[test]
fn resolve_seed_decorated_query_matches_bare_label() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true,
        "nodes": [
            {"id": "a", "label": "Foo", "source_file": "pkg/foo.py"},
            {"id": "b", "label": "FooBar", "source_file": "pkg/foobar.py"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    assert_eq!(resolve_seed(&graph, "Foo()"), Some("a".to_owned()));
}

// Mirrors: test_resolve_seed_bare_name_tie_still_returns_none
#[test]
fn resolve_seed_bare_name_tie_still_returns_none() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true,
        "nodes": [
            {"id": "a", "label": "dup()", "source_file": "pkg/one.py"},
            {"id": "b", "label": "dup()", "source_file": "pkg/two.py"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    assert_eq!(resolve_seed(&graph, "dup"), None);
}

// Mirrors: test_resolve_seed_matches_unicode_normalized_label
#[test]
fn resolve_seed_matches_unicode_normalized_label() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    // Label is stored NFC-composed: "í" is U+00ED.
    let payload = json!({
        "directed": true,
        "nodes": [
            {"id": "a", "label": "Auditor\u{00ed}a", "source_file": "pkg/auditoria.py"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    // Query is the NFD-decomposed form: "i" + U+0301 (combining acute accent).
    // It must still resolve to the NFC-stored label.
    assert_eq!(
        resolve_seed(&graph, "Auditori\u{0301}a"),
        Some("a".to_owned())
    );
}

// Mirrors: test_resolve_seed_preserves_distinct_accents
#[test]
fn resolve_seed_preserves_distinct_accents() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true,
        "nodes": [
            {"id": "a", "label": "resume", "source_file": "pkg/resume.py"},
            {"id": "b", "label": "r\u{00e9}sum\u{00e9}", "source_file": "pkg/resume_accented.py"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    assert_eq!(resolve_seed(&graph, "resume"), Some("a".to_owned()));
    assert_eq!(
        resolve_seed(&graph, "r\u{00e9}sum\u{00e9}"),
        Some("b".to_owned())
    );
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

// Mirrors: test_resolve_seed_source_file_path_prefers_file_level_node (#1503)
#[test]
fn resolve_seed_source_file_path_prefers_file_level_node() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "example_route_get", "label": "GET()",
             "source_file": "app/api/example/route.ts", "source_location": "L42"},
            {"id": "example_route", "label": "route.ts",
             "source_file": "app/api/example/route.ts", "source_location": "L1"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    // `load_graph` runs `build_from_json`, which re-keys non-AST nodes to the
    // full repo-relative path id (#1504); the L1 file node `example_route` here
    // becomes `app_api_example_route`. resolve_seed must still prefer it over the
    // L42 symbol that shares the source_file (#1503).
    let graph = load_graph(&path).expect("load");
    assert_eq!(
        resolve_seed(&graph, "app/api/example/route.ts"),
        Some("app_api_example_route".to_owned())
    );
}

// Mirrors: test_resolve_seed_source_file_trailing_slash_parity (#1503)
#[test]
fn resolve_seed_source_file_trailing_slash_parity() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "example_route_get", "label": "GET()",
             "source_file": "app/api/example/route.ts", "source_location": "L42"},
            {"id": "example_route", "label": "route.ts",
             "source_file": "app/api/example/route.ts", "source_location": "L1"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    // `load_graph` re-keys the L1 file node to its full repo-relative path id
    // (#1504): `example_route` → `app_api_example_route`. The trailing slash must
    // not change the match — resolve_seed still prefers that re-keyed file node.
    let graph = load_graph(&path).expect("load");
    assert_eq!(
        resolve_seed(&graph, "app/api/example/route.ts/"),
        Some("app_api_example_route".to_owned())
    );
}

// Mirrors: test_resolve_seed_source_file_ambiguous_no_file_node_returns_none (#1503)
#[test]
fn resolve_seed_source_file_ambiguous_no_file_node_returns_none() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("g.json");
    let payload = json!({
        "directed": true, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "a", "label": "handle_a()",
             "source_file": "pkg/handlers.py", "source_location": "L10"},
            {"id": "b", "label": "handle_b()",
             "source_file": "pkg/handlers.py", "source_location": "L20"},
        ],
        "links": [],
    });
    fs::write(&path, payload.to_string()).expect("write");
    let graph = load_graph(&path).expect("load");
    assert_eq!(resolve_seed(&graph, "pkg/handlers.py"), None);
}
