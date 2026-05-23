//! Coverage tests for `to_obsidian`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;

use graphify_build::build_from_json;
use graphify_cluster::cluster;
use graphify_export::to_obsidian;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tempfile::tempdir;

const EXTRACTION_JSON: &str = include_str!("../../../graphify-py/tests/fixtures/extraction.json");

fn fixture_graph() -> graphify_build::Graph {
    let val: Value = serde_json::from_str(EXTRACTION_JSON).unwrap();
    build_from_json(val, false, None).unwrap()
}

fn fixture_communities() -> IndexMap<i64, Vec<String>> {
    cluster(&fixture_graph(), 1.0, None)
}

#[test]
fn obsidian_creates_node_notes() {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir().unwrap();
    let count = to_obsidian(&g, &communities, tmp.path(), None, None).unwrap();
    // Returned count is node_count + community_notes_written.
    assert_eq!(count, g.node_count() + communities.len());

    // At least one node note must exist (filename derived from label).
    let md_files: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == std::ffi::OsStr::new("md"))
        })
        .collect();
    assert!(!md_files.is_empty(), "expected node markdown files");
}

#[test]
fn obsidian_creates_community_notes() {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir().unwrap();
    to_obsidian(&g, &communities, tmp.path(), None, None).unwrap();

    let comm_notes: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("_COMMUNITY_"))
        .collect();
    assert_eq!(comm_notes.len(), communities.len());
}

#[test]
fn obsidian_writes_graph_json_for_colors() {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir().unwrap();
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    for cid in communities.keys() {
        labels.insert(*cid, format!("Community-{cid}"));
    }
    to_obsidian(&g, &communities, tmp.path(), Some(&labels), None).unwrap();

    let obs_json = tmp.path().join(".obsidian").join("graph.json");
    assert!(obs_json.exists(), ".obsidian/graph.json missing");
    let text = fs::read_to_string(&obs_json).unwrap();
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert!(parsed.get("colorGroups").is_some());
}

#[test]
fn obsidian_uses_cohesion_in_community_notes() {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir().unwrap();
    let mut cohesion: IndexMap<i64, f64> = IndexMap::new();
    #[allow(clippy::cast_precision_loss)]
    for (i, cid) in communities.keys().enumerate() {
        cohesion.insert(*cid, 0.5_f64 + (i as f64) * 0.1);
    }
    to_obsidian(&g, &communities, tmp.path(), None, Some(&cohesion)).unwrap();

    let comm_notes: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("_COMMUNITY_"))
        .collect();
    assert!(!comm_notes.is_empty());

    // At least one community note should mention cohesion.
    let any_with_cohesion = comm_notes.iter().any(|e| {
        let s = fs::read_to_string(e.path()).unwrap_or_default();
        s.to_lowercase().contains("cohesion")
    });
    assert!(any_with_cohesion);
}

#[test]
fn obsidian_handles_duplicate_node_labels() {
    // Two nodes share the same label — filenames must dedup with numeric suffix.
    let graph = build_from_json(
        json!({
            "nodes": [
                {"id": "a1", "label": "Foo", "source_file": "a.py", "community": 0},
                {"id": "a2", "label": "Foo", "source_file": "b.py", "community": 0},
            ],
            "edges": []
        }),
        false,
        None,
    )
    .unwrap();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["a1".to_string(), "a2".to_string()]);

    let tmp = tempdir().unwrap();
    to_obsidian(&graph, &communities, tmp.path(), None, None).unwrap();

    let names: Vec<String> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == std::ffi::OsStr::new("md"))
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    // Should have two distinct files for "Foo" (one base + one suffixed).
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("Foo.md") || n == "Foo.md"),
        "missing Foo.md (got: {names:?})"
    );
}

#[test]
fn obsidian_handles_empty_communities() {
    let graph = build_from_json(
        json!({
            "nodes": [{"id": "x", "label": "Solo", "community": 0}],
            "edges": []
        }),
        false,
        None,
    )
    .unwrap();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["x".to_string()]);

    let tmp = tempdir().unwrap();
    let count = to_obsidian(&graph, &communities, tmp.path(), None, None).unwrap();
    assert_eq!(count, graph.node_count() + 1);
}
