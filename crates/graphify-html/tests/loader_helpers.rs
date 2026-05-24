//! Coverage tests for the small loader/helper functions in
//! `graphify_html::callflow::loader`.

#![allow(clippy::expect_used)]

use std::fs;

use graphify_html::callflow::{
    html_comment_text, infer_project_name, load_graph, load_labels, load_report,
    mermaid_section_id, node_mermaid_id, normalize_edge, normalize_node, safe_file_path,
    safe_filename, safe_mermaid_text, stable_ascii_id,
};
use indexmap::IndexMap;
use serde_json::json;

// ── safe_mermaid_text ───────────────────────────────────────────────────────

#[test]
fn safe_mermaid_text_strips_dangerous_chars() {
    let s = safe_mermaid_text("foo\"bar`baz#qux|");
    assert!(!s.contains('"'));
    assert!(!s.contains('`'));
    // Note: htmlescape may emit &amp; etc.; we just check raw chars were
    // sanitised before encoding.
    assert!(!s.contains("foo\""));
}

#[test]
fn safe_mermaid_text_replaces_arrow_sequences() {
    let s = safe_mermaid_text("a -> b ->> c --> d");
    assert!(s.contains(" to "));
    assert!(!s.contains("->"));
}

#[test]
fn safe_mermaid_text_collapses_whitespace() {
    let s = safe_mermaid_text("hello   world\n\nfoo");
    assert_eq!(s, "hello world foo");
}

#[test]
fn safe_mermaid_text_html_escapes() {
    let s = safe_mermaid_text("<b>x</b>");
    assert!(s.contains("&lt;"));
    assert!(s.contains("&gt;"));
}

// ── html_comment_text ───────────────────────────────────────────────────────

#[test]
fn html_comment_text_replaces_dashes() {
    let s = html_comment_text("foo -- bar");
    assert!(!s.contains("--"));
}

#[test]
fn html_comment_text_strips_newlines() {
    let s = html_comment_text("a\nb\nc");
    assert!(!s.contains('\n'));
}

// ── stable_ascii_id ─────────────────────────────────────────────────────────

#[test]
fn stable_ascii_id_is_deterministic() {
    let a = stable_ascii_id("Module::with::path", "node", 48);
    let b = stable_ascii_id("Module::with::path", "node", 48);
    assert_eq!(a, b);
}

#[test]
fn stable_ascii_id_handles_leading_digit() {
    let s = stable_ascii_id("123_starts_with_digit", "node", 48);
    assert!(s.starts_with("node_"));
}

#[test]
fn stable_ascii_id_handles_empty_slug() {
    // Pure punctuation collapses to empty.
    let s = stable_ascii_id("....", "node", 48);
    assert!(s.starts_with("node_"));
}

#[test]
fn stable_ascii_id_respects_limit() {
    let s = stable_ascii_id("very_long_name_that_exceeds_the_limit_for_sure", "n", 8);
    // The non-hash portion shouldn't exceed limit.
    let parts: Vec<&str> = s.rsplitn(2, '_').collect();
    let prefix = parts.last().expect("non-empty");
    assert!(prefix.len() <= 8, "prefix exceeded limit: {prefix}");
}

#[test]
fn node_mermaid_id_uses_node_prefix() {
    let s = node_mermaid_id("simple-id");
    assert!(s.contains("simple") || s.starts_with("node_"));
}

#[test]
fn mermaid_section_id_uppercases() {
    let s = mermaid_section_id("my-section");
    assert!(s.chars().any(|c| c.is_ascii_uppercase()));
}

// ── safe_file_path ──────────────────────────────────────────────────────────

#[test]
fn safe_file_path_returns_short_path_unchanged() {
    assert_eq!(safe_file_path("a/b.py"), "a/b.py");
}

#[test]
fn safe_file_path_keeps_last_three_components() {
    let p = safe_file_path("very/deep/path/to/source.py");
    assert_eq!(p, "path/to/source.py");
}

// ── safe_filename ───────────────────────────────────────────────────────────

#[test]
fn safe_filename_replaces_unsafe_chars() {
    assert_eq!(safe_filename("hello world!"), "hello-world");
}

#[test]
fn safe_filename_falls_back_to_project() {
    assert_eq!(safe_filename("!!!"), "project");
}

// ── normalize_node ──────────────────────────────────────────────────────────

#[test]
fn normalize_node_extracts_fields() {
    let raw = json!({
        "id": "n1",
        "label": "A",
        "source_file": "a.py",
        "file_type": "code",
        "community": 0
    });
    let m = raw.as_object().expect("object field");
    let n = normalize_node(m, 0);
    assert_eq!(n.id, "n1");
    assert_eq!(n.label, "A");
    assert_eq!(n.source_file, "a.py");
}

#[test]
fn normalize_node_uses_index_when_missing_id() {
    let raw = json!({"label": "no_id_node"});
    let m = raw.as_object().expect("object field");
    let n = normalize_node(m, 7);
    assert!(!n.id.is_empty());
}

// ── normalize_edge ──────────────────────────────────────────────────────────

#[test]
fn normalize_edge_basic() {
    let raw = json!({"source": "a", "target": "b", "relation": "calls"});
    let m = raw.as_object().expect("object field");
    let edge = normalize_edge(m, 0);
    assert!(edge.is_some());
    let edge = edge.expect("test invariant");
    assert_eq!(edge.source, "a");
    assert_eq!(edge.target, "b");
}

#[test]
fn normalize_edge_returns_none_for_missing_endpoints() {
    let raw = json!({"relation": "calls"});
    let m = raw.as_object().expect("object field");
    assert!(normalize_edge(m, 0).is_none());
}

// ── load_graph ──────────────────────────────────────────────────────────────

#[test]
fn load_graph_with_links() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("graph.json");
    fs::write(
        &path,
        r#"{
            "nodes": [{"id": "a", "label": "A", "source_file": "a.py"}],
            "links": [{"source": "a", "target": "a", "relation": "self"}]
        }"#,
    )
    .expect("test invariant");
    let g = load_graph(&path).expect("load_graph ok");
    assert_eq!(g.0.len(), 1);
}

#[test]
fn load_graph_with_edges_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("graph.json");
    fs::write(
        &path,
        r#"{
            "nodes": [{"id": "a", "label": "A", "source_file": "a.py"}],
            "edges": [{"source": "a", "target": "a", "relation": "self"}]
        }"#,
    )
    .expect("test invariant");
    let g = load_graph(&path).expect("load_graph ok");
    assert_eq!(g.0.len(), 1);
}

#[test]
fn load_graph_missing_file_errors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert!(load_graph(&tmp.path().join("nope.json")).is_err());
}

#[test]
fn load_graph_bad_json_errors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("graph.json");
    fs::write(&path, "{ not valid").expect("write fixture");
    assert!(load_graph(&path).is_err());
}

// ── load_labels / load_report ───────────────────────────────────────────────

#[test]
fn load_labels_returns_empty_for_missing() {
    assert!(load_labels(None).is_empty());
}

#[test]
fn load_labels_reads_object() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("labels.json");
    fs::write(&p, r#"{"0": "Foo", "1": "Bar"}"#).expect("write fixture");
    let labels = load_labels(Some(&p));
    assert_eq!(labels.get("0"), Some(&"Foo".to_string()));
}

#[test]
fn load_report_returns_empty_for_missing() {
    assert!(load_report(None).is_empty());
}

#[test]
fn load_report_reads_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let p = tmp.path().join("report.md");
    fs::write(&p, "# Report\nhello").expect("write fixture");
    let r = load_report(Some(&p));
    assert!(r.contains("Report"));
}

// ── infer_project_name ─────────────────────────────────────────────────────

#[test]
fn infer_project_name_from_path() {
    let meta: IndexMap<String, serde_json::Value> = IndexMap::new();
    let p = std::path::PathBuf::from("/some/project/path/graphify-out/graph.json");
    let n = infer_project_name(&p, &meta);
    assert!(!n.is_empty());
}
