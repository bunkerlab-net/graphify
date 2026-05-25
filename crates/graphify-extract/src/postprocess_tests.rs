//! Unit tests for [`crate::postprocess`].
//!
//! Extracted from the inline `#[cfg(test)] mod tests { ... }` block
//! that used to live at the bottom of `postprocess.rs`. Behaviour is
//! unchanged; this layout matches the workspace convention that
//! tests live in dedicated `_tests.rs` (or `tests/parity.rs`) files.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

use super::*;
use crate::types::Edge;

fn n(id: &str, label: &str, file_type: &str, source_file: &str) -> Node {
    Node {
        id: id.to_string(),
        label: label.to_string(),
        file_type: file_type.to_string(),
        source_file: source_file.to_string(),
        source_location: None,
    }
}

fn e(src: &str, tgt: &str, relation: &str) -> Edge {
    Edge {
        source: src.to_string(),
        target: tgt.to_string(),
        relation: relation.to_string(),
        confidence: String::new(),
        source_file: String::new(),
        source_location: None,
        weight: 0.0,
        context: None,
        confidence_score: None,
    }
}

#[test]
fn node_label_key_strips_punctuation_and_lowercases() {
    assert_eq!(node_label_key("Foo.Bar!"), "foobar");
    assert_eq!(node_label_key("  Hello World  "), "helloworld");
}

#[test]
fn is_type_like_definition_rejects_method_signatures() {
    let method = n("m", "Foo()", "code", "a.py");
    assert!(!is_type_like_definition(&method));
}

#[test]
fn is_type_like_definition_rejects_dotted_labels() {
    let dotted = n("d", "a.b.c", "code", "a.py");
    assert!(!is_type_like_definition(&dotted));
}

#[test]
fn is_type_like_definition_accepts_plain_class() {
    let cls = n("c", "Foo", "code", "a.py");
    assert!(is_type_like_definition(&cls));
}

#[test]
fn rewire_unique_stub_collapses_to_single_real() {
    let mut nodes = vec![
        n("stub", "Foo", "code", ""),
        n("real", "Foo", "code", "a.py"),
        n("user", "Bar", "code", "b.py"),
    ];
    let mut edges = vec![e("user", "stub", "inherits")];
    rewire_unique_stub_nodes(&mut nodes, &mut edges);
    // stub node should be dropped, edge should now target `real`.
    assert!(nodes.iter().all(|n| n.id != "stub"));
    assert_eq!(edges[0].target, "real");
}

#[test]
fn rewire_skips_when_multiple_real_definitions_match() {
    let mut nodes = vec![
        n("stub", "Foo", "code", ""),
        n("real_a", "Foo", "code", "a.py"),
        n("real_b", "Foo", "code", "b.py"),
    ];
    let mut edges = vec![e("user", "stub", "inherits")];
    rewire_unique_stub_nodes(&mut nodes, &mut edges);
    // ambiguous — stub should remain, edge untouched.
    assert!(nodes.iter().any(|n| n.id == "stub"));
    assert_eq!(edges[0].target, "stub");
}
