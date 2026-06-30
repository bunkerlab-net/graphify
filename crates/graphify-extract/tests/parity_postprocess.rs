//! Parity tests for the corpus-level post-processing passes
//! (`disambiguate_colliding_node_ids`, `rewire_unique_stub_nodes`).
#![allow(clippy::expect_used)]

use std::path::Path;

use graphify_extract::postprocess::{
    disambiguate_colliding_node_ids, rewire_unique_stub_nodes, source_key,
};
use graphify_extract::types::{Edge, Node, RawCall};

fn n(id: &str, label: &str, source_file: &str) -> Node {
    Node {
        id: id.to_string(),
        label: label.to_string(),
        file_type: "code".to_string(),
        source_file: source_file.to_string(),
        source_location: None,
        metadata: None,
        origin_file: None,
    }
}

fn e(src: &str, tgt: &str, source_file: &str, relation: &str) -> Edge {
    Edge {
        source: src.to_string(),
        target: tgt.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: source_file.to_string(),
        source_location: None,
        weight: 1.0,
        context: None,
        confidence_score: None,
        external: false,
    }
}

#[allow(clippy::similar_names)] // domain-canonical caller/callee terminology
fn raw(caller: &str, callee: &str, source_file: &str) -> RawCall {
    RawCall {
        caller_nid: caller.to_string(),
        callee: callee.to_string(),
        is_member_call: false,
        source_file: source_file.to_string(),
        source_location: String::new(),
        receiver: None,
        receiver_type: None,
    }
}

#[test]
fn source_key_returns_empty_for_empty_input() {
    assert_eq!(source_key("", Path::new(".")), "");
}

#[test]
fn source_key_returns_input_when_no_canonical_match() {
    // A non-existent file falls through to the lossless path.
    assert_eq!(
        source_key("does/not/exist.py", Path::new(".")),
        "does/not/exist.py"
    );
}

#[test]
fn disambiguate_rewrites_only_colliding_ids() {
    // Two `Program.cs` files in different directories produce the same ID
    // by default. The disambiguation pass should rename both, rewrite the
    // edge endpoints, and leave non-colliding IDs untouched.
    let mut nodes = vec![
        n("program", "Program", "apps/api/Program.cs"),
        n("program", "Program", "tools/api/Program.cs"),
        n("unique", "Helper", "tools/api/Helper.cs"),
    ];
    let mut edges = vec![e("unique", "program", "tools/api/Helper.cs", "calls")];
    let mut raw_calls = vec![raw("unique", "Program", "tools/api/Helper.cs")];
    disambiguate_colliding_node_ids(&mut nodes, &mut edges, &mut raw_calls, Path::new("."));
    let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        ids.iter().filter(|id| **id == "program").count() == 0,
        "no node should retain the colliding ID: {ids:?}"
    );
    // The non-colliding ID is untouched.
    assert!(ids.contains(&"unique"));
}

#[test]
fn disambiguate_leaves_single_occurrence_ids_alone() {
    let mut nodes = vec![n("foo", "Foo", "a.py"), n("bar", "Bar", "b.py")];
    let mut edges = vec![e("foo", "bar", "a.py", "calls")];
    let mut raw_calls: Vec<RawCall> = Vec::new();
    let original_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    disambiguate_colliding_node_ids(&mut nodes, &mut edges, &mut raw_calls, Path::new("."));
    let new_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    assert_eq!(new_ids, original_ids);
}

#[test]
fn rewire_collapses_stub_onto_unique_real_definition() {
    let mut nodes = vec![
        n("stub", "Foo", ""),
        n("real", "Foo", "a.py"),
        n("user", "Bar", "b.py"),
    ];
    let mut edges = vec![e("user", "stub", "b.py", "inherits")];
    rewire_unique_stub_nodes(&mut nodes, &mut edges);
    assert!(nodes.iter().all(|n| n.id != "stub"));
    assert_eq!(edges[0].target, "real");
}

#[test]
fn rewire_keeps_stub_when_real_matches_are_ambiguous() {
    let mut nodes = vec![
        n("stub", "Foo", ""),
        n("real_a", "Foo", "a.py"),
        n("real_b", "Foo", "b.py"),
    ];
    let mut edges = vec![e("user", "stub", "u.py", "inherits")];
    rewire_unique_stub_nodes(&mut nodes, &mut edges);
    assert!(
        nodes.iter().any(|n| n.id == "stub"),
        "ambiguous match must leave stub in place"
    );
    assert_eq!(edges[0].target, "stub");
}

#[test]
fn rewire_does_not_rewire_to_method_signature() {
    // A node labelled `Foo()` is a method, not a type-like definition.
    // Stubs for class `Foo` must not be rewired to it.
    let mut nodes = vec![n("stub", "Foo", ""), n("method", "Foo()", "a.py")];
    let mut edges = vec![e("user", "stub", "u.py", "inherits")];
    rewire_unique_stub_nodes(&mut nodes, &mut edges);
    assert!(nodes.iter().any(|n| n.id == "stub"));
    assert_eq!(edges[0].target, "stub");
}

#[test]
fn rewire_does_not_rewire_to_dotted_label() {
    // `pkg.Foo` is a qualified reference, not a type-like definition.
    let mut nodes = vec![n("stub", "Foo", ""), n("qual", "pkg.Foo", "a.py")];
    let mut edges = vec![e("user", "stub", "u.py", "inherits")];
    rewire_unique_stub_nodes(&mut nodes, &mut edges);
    assert!(nodes.iter().any(|n| n.id == "stub"));
}

#[test]
fn header_remap_skips_non_c_family_importers() {
    // #1475 parity-bug fix: a header-variant repoint applies only to a C-family
    // importer's `#include`. A Python `imports_from` whose target id merely
    // collides with a C header must NOT be silently rewritten to the header.
    let mut nodes = vec![
        n("foo", "foo", "src/foo.py"),    // python module
        n("foo", "foo", "include/foo.h"), // C header, same bare id
    ];
    let mut edges = vec![
        e("consumer", "foo", "src/consumer.py", "imports_from"), // non-C importer
        e("caller", "foo", "lib/util.c", "imports_from"),        // C importer
    ];
    let mut raw_calls: Vec<RawCall> = Vec::new();
    disambiguate_colliding_node_ids(&mut nodes, &mut edges, &mut raw_calls, Path::new("."));
    let header_id = nodes
        .iter()
        .find(|nd| nd.source_file == "include/foo.h")
        .map(|nd| nd.id.clone())
        .expect("header node present");
    assert_ne!(
        header_id, "foo",
        "header should be salted away from the bare id"
    );
    // A C-family `#include` resolves to the header variant...
    assert_eq!(
        edges[1].target, header_id,
        "a C `#include` should resolve to the header variant"
    );
    // ...while a non-C importer's edge must NOT be redirected to the header. We
    // assert only the negative here: the salt remap is keyed by the importer's own
    // file, and `consumer.py` matches neither colliding definition, so the pass
    // has no information to resolve a bare ambiguous import to the Python module.
    // Pinning a positive target would either codify a dangling id or assume a
    // module-inference step this pass (and the graphify-py reference) never does.
    assert_ne!(
        edges[0].target, header_id,
        "a non-C import must not be repointed at the header variant"
    );
}

#[test]
fn salted_id_does_not_collide_with_existing_node() {
    // #1522 hardening: when a salted id would equal an id already occupied
    // outside the collision group, it must be disambiguated further so no two
    // nodes share an id (graphify-py only de-dupes within the group).
    let naive = graphify_extract::make_id(&["src/a/foo.py", "foo"]);
    let mut nodes = vec![
        n("foo", "foo", "src/a/foo.py"),
        n("foo", "foo", "src/b/foo.py"),
        n(&naive, "other", "src/c/other.py"), // already occupies the naive salt
    ];
    let mut edges: Vec<Edge> = Vec::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    disambiguate_colliding_node_ids(&mut nodes, &mut edges, &mut raw_calls, Path::new("."));
    let ids: Vec<&str> = nodes.iter().map(|nd| nd.id.as_str()).collect();
    let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "all node ids must be distinct: {ids:?}"
    );
}

#[test]
fn salted_ids_unique_across_colliding_groups() {
    // Two distinct old ids that normalise to the same salted form under the same
    // source key (`foo-bar`/`foo_bar` both -> `shared_rb_foo_bar`): a live
    // minted-set keeps the two `shared.rb` nodes from reusing one id. Without it
    // the second group reassigns the first group's salted id.
    let mut nodes = vec![
        n("foo-bar", "FooBar", "shared.rb"), // group "foo-bar"
        n("foo-bar", "FooBar", "a.rb"),      // makes "foo-bar" ambiguous
        n("foo_bar", "FooBar", "shared.rb"), // group "foo_bar"
        n("foo_bar", "FooBar", "b.rb"),      // makes "foo_bar" ambiguous
    ];
    let mut edges: Vec<Edge> = Vec::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    disambiguate_colliding_node_ids(&mut nodes, &mut edges, &mut raw_calls, Path::new("."));
    let ids: Vec<&str> = nodes.iter().map(|nd| nd.id.as_str()).collect();
    let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "all node ids must be distinct: {ids:?}"
    );
}
