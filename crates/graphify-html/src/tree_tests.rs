//! Unit tests for [`crate::tree`].
//!
//! Focuses on the small, self-contained HTML-emission helpers; the full
//! `write_tree_html` round-trip is exercised in `tests/parity.rs`.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

use super::*;
use graphify_build::{Graph, GraphKind};
use indexmap::IndexMap;
use serde_json::Value;

/// Build a single-node graph with the given `id`/`label`/`source_file` tuple.
///
/// Centralised here so each test reads as one assertion rather than ten
/// lines of fixture wiring.
fn graph_with_node(id: &str, label: &str, source_file: &str) -> Graph {
    let mut g = Graph::new(GraphKind::Graph);
    let mut attrs = IndexMap::new();
    attrs.insert("label".to_owned(), Value::String(label.to_owned()));
    attrs.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    attrs.insert("file_type".to_owned(), Value::String("code".to_owned()));
    g.add_node(id, attrs);
    g
}

/// When the graph has no nodes, the placeholder tree carries the
/// `(empty graph)` label so the UI can render a friendly message.
#[test]
fn empty_graph_returns_placeholder() {
    let g = Graph::new(GraphKind::Graph);
    let tree = build_tree(&g, None, DEFAULT_MAX_CHILDREN, None);
    assert_eq!(tree["name"], "(empty graph)");
    assert_eq!(tree["total_count"], 0);
}

/// A single source file must surface in the tree both by its basename and by
/// the symbol it contains.
#[test]
fn single_node_appears_in_tree() {
    let g = graph_with_node("n1", "my_func", "/proj/src/foo.py");
    let tree = build_tree(&g, Some(Path::new("/proj")), DEFAULT_MAX_CHILDREN, None);
    let s = serde_json::to_string(&tree).expect("serialise JSON");
    assert!(s.contains("foo.py"), "expected foo.py: {s}");
    assert!(s.contains("my_func"), "expected my_func: {s}");
}

/// HTML-injection guard — angle brackets in the title or header must be
/// escaped before they appear in the emitted document.
#[test]
fn emit_tree_html_escapes_title_and_header() {
    let tree = serde_json::json!({"name": "x", "total_count": 1, "children": []});
    let html = emit_tree_html(&tree, "<evil>", "<also evil>", 100, 100);
    assert!(html.contains("&lt;evil&gt;"), "title not escaped");
    assert!(html.contains("&lt;also evil&gt;"), "header not escaped");
    assert!(!html.contains("<evil>"), "raw title found");
    assert!(!html.contains("<also evil>"), "raw header found");
}

/// `</script>` sequences inside the inline JSON data island would close
/// the surrounding `<script>` block early, so `emit_tree_html` neutralises
/// them. The full HTML still contains `</script>` as the legitimate close
/// tag for the data block itself, so the assertion only checks the
/// JSON-quoted form that would only appear if neutralisation failed, and
/// also asserts the escaped form is present.
#[test]
fn emit_tree_html_neutralises_script_close() {
    let tree = serde_json::json!({"name": "</script>", "total_count": 1, "children": []});
    let html = emit_tree_html(&tree, "t", "h", 100, 100);
    assert!(
        !html.contains("\"</script>\""),
        "raw </script> survived in JSON string literal"
    );
    assert!(
        html.contains("<\\/script>"),
        "expected neutralised <\\/script> in JSON"
    );
}
