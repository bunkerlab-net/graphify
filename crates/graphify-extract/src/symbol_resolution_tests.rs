//! Unit tests for [`crate::symbol_resolution`].
//!
//! Extracted from the inline `#[cfg(test)] mod tests { ... }` block
//! that used to live at the bottom of `symbol_resolution.rs`. Behaviour is
//! unchanged; this layout matches the workspace convention that
//! tests live in dedicated `_tests.rs` (or `tests/parity.rs`) files.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

use super::*;

fn n(id: &str, label: &str, file_type: &str) -> Node {
    Node {
        id: id.to_string(),
        label: label.to_string(),
        file_type: file_type.to_string(),
        source_file: "src.py".to_string(),
        source_location: None,
        metadata: None,
    }
}

#[test]
fn normalise_strips_parens_and_dot() {
    assert_eq!(normalise_callable_label("Foo()"), "foo");
    assert_eq!(normalise_callable_label(".bar"), "bar");
    assert_eq!(normalise_callable_label("  Baz  "), "baz");
}

#[test]
fn node_is_resolvable_rejects_filename_labels() {
    assert!(!node_is_resolvable_symbol(&n("a", "module.py", "code")));
    assert!(!node_is_resolvable_symbol(&n("b", "thing.ts", "code")));
}

#[test]
fn node_is_resolvable_rejects_non_code_file_type() {
    assert!(!node_is_resolvable_symbol(&n("a", "Foo", "document")));
}

#[test]
fn node_is_resolvable_accepts_plain_code_label() {
    assert!(node_is_resolvable_symbol(&n("a", "Foo", "code")));
}

#[test]
fn build_label_index_groups_by_normalised_label() {
    let nodes = vec![
        n("a", "Foo()", "code"),
        n("b", "Foo", "code"),
        n("c", "Bar", "code"),
    ];
    let index = build_label_index(&nodes);
    assert_eq!(index["foo"], vec!["a".to_string(), "b".to_string()]);
    assert_eq!(index["bar"], vec!["c".to_string()]);
}

#[test]
fn existing_edge_pairs_includes_relation() {
    let edges = vec![
        Edge {
            source: "a".to_string(),
            target: "b".to_string(),
            relation: "contains".to_string(),
            confidence: String::new(),
            source_file: String::new(),
            source_location: None,
            weight: 0.0,
            context: None,
            confidence_score: None,
        },
        Edge {
            source: "a".to_string(),
            target: "b".to_string(),
            relation: "calls".to_string(),
            confidence: String::new(),
            source_file: String::new(),
            source_location: None,
            weight: 0.0,
            context: None,
            confidence_score: None,
        },
    ];
    let pairs = existing_edge_pairs(&edges);
    assert!(pairs.contains(&(
        "a".to_string(),
        "b".to_string(),
        "contains".to_string(),
        String::new()
    )));
    assert!(pairs.contains(&(
        "a".to_string(),
        "b".to_string(),
        "calls".to_string(),
        String::new()
    )));
}

#[test]
fn existing_edge_pairs_distinguishes_context() {
    // Two `references` edges between the same nodes but with different
    // contexts must both survive deduplication. Ports the dedup-key change
    // in graphify-py `_apply_symbol_resolution_facts` (ab4e542).
    let edges = vec![
        Edge {
            source: "a".to_string(),
            target: "b".to_string(),
            relation: "references".to_string(),
            confidence: String::new(),
            source_file: String::new(),
            source_location: None,
            weight: 0.0,
            context: Some("parameter_type".to_string()),
            confidence_score: None,
        },
        Edge {
            source: "a".to_string(),
            target: "b".to_string(),
            relation: "references".to_string(),
            confidence: String::new(),
            source_file: String::new(),
            source_location: None,
            weight: 0.0,
            context: Some("return_type".to_string()),
            confidence_score: None,
        },
    ];
    let pairs = existing_edge_pairs(&edges);
    assert!(pairs.contains(&(
        "a".to_string(),
        "b".to_string(),
        "references".to_string(),
        "parameter_type".to_string()
    )));
    assert!(pairs.contains(&(
        "a".to_string(),
        "b".to_string(),
        "references".to_string(),
        "return_type".to_string()
    )));
    assert_eq!(pairs.len(), 2);
}
