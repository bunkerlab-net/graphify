//! Tests that exercise non-ASCII identifiers (emoji, non-Latin scripts) through
//! the major pipeline stages: extraction, graph build, JSON serialization,
//! Mermaid HTML, and ID generation.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_character_iteration
)]

use std::fs;

use graphify_build::build_from_json;
use graphify_export::{to_json, to_obsidian};
use graphify_extract::{extract_python, make_id, make_id1};
use graphify_html::callflow::{safe_filename, safe_mermaid_text, stable_ascii_id};
use indexmap::IndexMap;
use serde_json::json;
use tempfile::tempdir;

// ── make_id with non-ASCII ──────────────────────────────────────────────────

#[test]
fn make_id_preserves_non_latin() {
    // After NFKC + casefold, Cyrillic still survives.
    let id = make_id1("Кириллица");
    assert!(
        id.chars().any(|c| !c.is_ascii()),
        "expected non-ascii preserved, got {id}"
    );
}

#[test]
fn make_id_handles_emoji() {
    // \w in regex includes Unicode by default in `regex` crate — emoji are
    // *not* `\w`, so they should be replaced with underscores. Just verify
    // it doesn't panic and produces a valid string.
    let id = make_id(&["foo🚀bar"]);
    assert_ne!(id, "");
}

#[test]
fn make_id_nfkc_normalises() {
    // Fullwidth characters fold to ASCII via NFKC.
    let a = make_id1("ＡＢＣ");
    let b = make_id1("ABC");
    assert_eq!(a, b, "NFKC should fold fullwidth to ASCII");
}

#[test]
fn make_id_japanese_round_trip() {
    let a = make_id1("関数");
    let b = make_id1("関数");
    assert_eq!(a, b);
}

// ── extract from a file with non-ASCII identifiers ─────────────────────────

#[test]
fn extract_python_with_non_ascii_identifiers() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("Главный.py");
    fs::write(
        &path,
        "class Класс:\n    def метод(self):\n        return '日本語'\n",
    )
    .unwrap();

    let result = extract_python(&path);
    assert!(result.error.is_none(), "extract failed: {:?}", result.error);
    assert!(!result.nodes.is_empty());
    // At least one label should preserve the non-Latin characters.
    let has_non_ascii = result
        .nodes
        .iter()
        .any(|n| n.label.chars().any(|c| !c.is_ascii()));
    assert!(has_non_ascii, "no non-ascii labels survived");
}

#[test]
fn extract_python_with_emoji_in_string_literal() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("emoji_strings.py");
    fs::write(
        &path,
        "def go():\n    return '🚀🎯💎'\n\nclass Rocket:\n    pass\n",
    )
    .unwrap();
    let result = extract_python(&path);
    assert!(result.error.is_none());
    assert!(!result.nodes.is_empty());
}

// ── build + serialize graph with unicode labels ────────────────────────────

#[test]
fn graph_build_and_serialize_with_unicode_labels() {
    let graph = build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "Класс", "source_file": "a.py", "file_type": "code", "community": 0},
                {"id": "n2", "label": "関数", "source_file": "b.py", "file_type": "code", "community": 0},
                {"id": "n3", "label": "function_🚀", "source_file": "c.py", "file_type": "code", "community": 1},
            ],
            "edges": [
                {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"},
                {"source": "n2", "target": "n3", "relation": "imports", "confidence": "EXTRACTED"}
            ]
        }),
        false,
        None,
    )
    .unwrap();

    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["n1".into(), "n2".into()]);
    communities.insert(1, vec!["n3".into()]);

    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.json");
    to_json(&graph, &communities, &out, true, None, None).unwrap();
    let text = fs::read_to_string(&out).unwrap();
    // JSON serialization may escape with \uXXXX OR emit literal UTF-8.
    // Either is valid JSON — check that round-tripping recovers the strings.
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let labels: Vec<String> = parsed["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| {
            n.get("label")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect();
    assert!(labels.iter().any(|l| l == "Класс"));
    assert!(labels.iter().any(|l| l == "関数"));
    assert!(labels.iter().any(|l| l.contains('🚀')));
}

// ── obsidian export with unicode labels ────────────────────────────────────

#[test]
fn obsidian_handles_unicode_node_labels() {
    let graph = build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "Класс_тест", "source_file": "a.py", "community": 0},
                {"id": "n2", "label": "関数_テスト", "source_file": "b.py", "community": 0}
            ],
            "edges": []
        }),
        false,
        None,
    )
    .unwrap();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["n1".into(), "n2".into()]);

    let tmp = tempdir().unwrap();
    let count = to_obsidian(&graph, &communities, tmp.path(), None, None);
    assert!(count.is_ok(), "obsidian failed: {:?}", count.err());
}

// ── safe_mermaid_text + safe_filename with unicode ─────────────────────────

#[test]
fn safe_mermaid_text_preserves_unicode_words() {
    // Mermaid label text accepts arbitrary unicode after sanitisation.
    let s = safe_mermaid_text("Класс関数🚀");
    assert!(s.chars().any(|c| !c.is_ascii()), "got: {s}");
}

#[test]
fn safe_filename_replaces_non_ascii_with_dash() {
    let s = safe_filename("Класс関数");
    // Whole string is non-ASCII → collapses to a dash → returns "project" fallback.
    assert_eq!(s, "project");
}

#[test]
fn stable_ascii_id_collapses_non_ascii_but_is_deterministic() {
    let a = stable_ascii_id("Класс", "node", 32);
    let b = stable_ascii_id("Класс", "node", 32);
    assert_eq!(a, b, "non-ASCII input should still be deterministic");
    // The slug part should collapse to "node_" since no ASCII alnum survived.
    assert!(a.starts_with("node_"));
}

// ── path with non-ASCII filename ───────────────────────────────────────────

#[test]
fn extract_works_with_non_ascii_filename() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("日本語ファイル.py");
    fs::write(&path, "def foo():\n    pass\n").unwrap();
    let result = extract_python(&path);
    assert!(result.error.is_none(), "extract failed: {:?}", result.error);
    assert!(!result.nodes.is_empty());
}
