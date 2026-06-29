//! Parity tests for node-id collision salting (#1522), ported from
//! `graphify-py/tests/test_extract.py`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::case_sensitive_file_extension_comparisons
)]

use std::collections::HashSet;
use std::path::Path;

use graphify_extract::{extract, file_stem, make_id};
use serde_json::Value;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create_dir_all");
    std::fs::write(path, text).expect("write");
}

#[test]
fn separator_collision_paths_get_distinct_ids() {
    // Two distinct paths whose only difference is a separator-vs-punctuation swap
    // (`foo/bar_baz.py` vs `foo_bar/baz.py`) normalize to the same stem; the
    // disambiguation pass salts the colliders with a stable path hash so they stay
    // distinct instead of silently merging (#1522).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("foo/bar_baz.py"), "class Widget:\n    pass\n");
    write(&root.join("foo_bar/baz.py"), "class Gadget:\n    pass\n");

    let out = extract(
        &[root.join("foo/bar_baz.py"), root.join("foo_bar/baz.py")],
        Some(root),
    );
    let file_ids: HashSet<&str> = out
        .nodes
        .iter()
        .filter(|n| {
            n.get("label")
                .and_then(Value::as_str)
                .is_some_and(|l| l.ends_with(".py"))
        })
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    let file_count = out
        .nodes
        .iter()
        .filter(|n| {
            n.get("label")
                .and_then(Value::as_str)
                .is_some_and(|l| l.ends_with(".py"))
        })
        .count();
    assert_eq!(file_count, 2, "both .py files must survive as nodes");
    assert_eq!(
        file_ids.len(),
        2,
        "file ids must stay distinct: {file_ids:?}"
    );
}

#[test]
fn non_colliding_path_id_is_not_salted() {
    // The collision hash must touch only actual colliders — a path with no
    // collision keeps its plain full-path stem id (no hash suffix).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("src/auth/session.py"),
        "class Session:\n    pass\n",
    );

    let out = extract(&[root.join("src/auth/session.py")], Some(root));
    let file_id = out
        .nodes
        .iter()
        .find(|n| n.get("source_location").and_then(Value::as_str) == Some("L1"))
        .and_then(|n| n.get("id").and_then(Value::as_str))
        .expect("file node with L1 source_location");
    assert_eq!(file_id, "src_auth_session");
    assert_eq!(
        file_id,
        make_id(&[&file_stem(Path::new("src/auth/session.py"))])
    );
}

#[test]
fn origin_file_is_not_serialized_into_extract_output() {
    // origin_file is an internal disambiguation hint (#1462) consumed only by the
    // colliding-id pass; it must not survive into the returned nodes (#1516),
    // where it would ship an absolute, machine-specific path (#555, #932).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("pkg/a.py"),
        "from pathlib import Path\ndef use_a(p: Path):\n    return p\n",
    );
    write(
        &root.join("pkg/b.py"),
        "from pathlib import Path\ndef use_b(p: Path):\n    return p\n",
    );
    let out = extract(&[root.join("pkg/a.py"), root.join("pkg/b.py")], Some(root));

    // The internal field is gone from every node...
    assert!(
        out.nodes.iter().all(|n| n.get("origin_file").is_none()),
        "origin_file leaked into output nodes"
    );
    // ...so no node leaks the absolute sandbox path in any string value.
    let root_str = root.to_string_lossy();
    let leaked: Vec<_> = out
        .nodes
        .iter()
        .flat_map(|n| n.iter())
        .filter(|(_, v)| v.as_str().is_some_and(|s| s.contains(root_str.as_ref())))
        .collect();
    assert!(
        leaked.is_empty(),
        "absolute paths leaked into nodes: {leaked:?}"
    );
    // ...yet the colliding-id pass still kept the two cross-file Path stubs distinct.
    let path_ids: HashSet<&str> = out
        .nodes
        .iter()
        .filter(|n| n.get("label").and_then(Value::as_str) == Some("Path"))
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        path_ids.len(),
        2,
        "two distinct Path stubs expected: {path_ids:?}"
    );
}

#[test]
fn go_imported_type_stubs_do_not_collide_across_source_files() {
    // #1462 for the dedicated extractors: same-label cross-file stubs must stay
    // distinct per file while keeping source_file empty so the #1402 rewire still
    // collapses them onto a real definition when one exists.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        &root.join("a/use_a.go"),
        "package a\n\nimport \"ext\"\n\nfunc UseA(w ext.Widget) {}\n",
    );
    write(
        &root.join("b/use_b.go"),
        "package b\n\nimport \"ext\"\n\nfunc UseB(w ext.Widget) {}\n",
    );
    let out = extract(
        &[root.join("a/use_a.go"), root.join("b/use_b.go")],
        Some(root),
    );
    let widgets: Vec<(&str, &str)> = out
        .nodes
        .iter()
        .filter(|n| n.get("label").and_then(Value::as_str) == Some("Widget"))
        .filter_map(|n| {
            Some((
                n.get("id")?.as_str()?,
                n.get("source_file").and_then(Value::as_str).unwrap_or(""),
            ))
        })
        .collect();
    assert_eq!(widgets.len(), 2, "expected 2 Widget stubs: {widgets:?}");
    assert_eq!(
        widgets
            .iter()
            .map(|(id, _)| *id)
            .collect::<HashSet<_>>()
            .len(),
        2,
        "Widget stub ids must be distinct: {widgets:?}"
    );
    assert!(
        widgets.iter().all(|(_, sf)| sf.is_empty()),
        "Widget stubs must stay sourceless: {widgets:?}"
    );
}
