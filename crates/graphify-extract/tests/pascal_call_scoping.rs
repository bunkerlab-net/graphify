//! Scoped call resolution in the Pascal/Delphi extractor (#1739).
//!
//! Ports the regex-fallback cases of
//! `graphify-py/tests/test_pascal_call_scoping.py` (the Rust extractor is
//! regex-only — tree-sitter-pascal is not on crates.io). Before the fix, calls
//! resolved via a single file-wide `{method_name: node_id}` dict with no class
//! scoping, so two unrelated classes declaring a same-named method silently
//! collapsed onto whichever was inserted last. Resolution is now scoped: own
//! class → ancestor chain → file-level free function → unambiguous file-wide.
//! (The cross-file inherited-call case lives in `pascal_resolution.rs`.)
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use graphify_extract::{FileResult, extract_pascal};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The single node id labelled `class_label`.
fn class_node_id(r: &FileResult, class_label: &str) -> String {
    let matches: Vec<&str> = r
        .nodes
        .iter()
        .filter(|n| n.label == class_label)
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one node labelled {class_label:?}, got {matches:?}"
    );
    matches[0].to_string()
}

/// The node id of `class_label`'s `method_label` method (via a `method` edge).
fn method_node_id(r: &FileResult, class_label: &str, method_label: &str) -> String {
    let class_id = class_node_id(r, class_label);
    for e in &r.edges {
        if e.relation == "method"
            && e.source == class_id
            && let Some(node) = r.nodes.iter().find(|n| n.id == e.target)
            && node.label == method_label
        {
            return node.id.clone();
        }
    }
    panic!("no method edge {class_label}.{method_label} found");
}

/// `true` when a `calls` edge `src_id -> tgt_id` exists.
fn has_call(r: &FileResult, src_id: &str, tgt_id: &str) -> bool {
    r.edges
        .iter()
        .any(|e| e.relation == "calls" && e.source == src_id && e.target == tgt_id)
}

#[test]
fn calls_scoped_to_own_class() {
    let r = extract_pascal(&fixtures().join("sample_scoped_calls.pas"));
    let first_configure = method_node_id(&r, "TFirstWidget", "Configure()");
    let first_reset = method_node_id(&r, "TFirstWidget", "Reset()");
    assert!(has_call(&r, &first_configure, &first_reset));
}

#[test]
fn calls_do_not_cross_unrelated_classes() {
    let r = extract_pascal(&fixtures().join("sample_scoped_calls.pas"));
    let first_configure = method_node_id(&r, "TFirstWidget", "Configure()");
    let second_reset = method_node_id(&r, "TSecondWidget", "Reset()");
    assert!(
        !has_call(&r, &first_configure, &second_reset),
        "TFirstWidget.Configure must not resolve Reset() to the unrelated \
         TSecondWidget.Reset"
    );
}

#[test]
fn calls_scoped_other_direction() {
    let r = extract_pascal(&fixtures().join("sample_scoped_calls.pas"));
    let second_configure = method_node_id(&r, "TSecondWidget", "Configure()");
    let second_reset = method_node_id(&r, "TSecondWidget", "Reset()");
    let first_reset = method_node_id(&r, "TFirstWidget", "Reset()");
    assert!(has_call(&r, &second_configure, &second_reset));
    assert!(
        !has_call(&r, &second_configure, &first_reset),
        "TSecondWidget.Configure must not resolve Reset() to the unrelated \
         TFirstWidget.Reset"
    );
}

#[test]
fn calls_resolve_via_ancestor_chain() {
    // Same-file inheritance: the per-file scoped resolver walks the `inherits`
    // chain, so TDerivedWidget.Run resolves the inherited Prepare() to
    // TBaseWidget.Prepare.
    let r = extract_pascal(&fixtures().join("sample_scoped_calls.pas"));
    let derived_run = method_node_id(&r, "TDerivedWidget", "Run()");
    let base_prepare = method_node_id(&r, "TBaseWidget", "Prepare()");
    assert!(
        has_call(&r, &derived_run, &base_prepare),
        "TDerivedWidget.Run should resolve inherited Prepare() to TBaseWidget.Prepare"
    );
}
