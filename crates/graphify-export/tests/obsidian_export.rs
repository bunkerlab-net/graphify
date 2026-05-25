//! Coverage tests for `to_obsidian`.

use std::fs;

use graphify_build::build_from_json;
use graphify_cluster::cluster;
use graphify_export::to_obsidian;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const EXTRACTION_JSON: &str = include_str!("../../../graphify-py/tests/fixtures/extraction.json");

// The fixture helpers panic on failure rather than returning `Result` — the
// fixture file is bundled into the test binary, so any parse/build error
// reflects a programmer mistake in this crate, not runtime input.
#[allow(clippy::expect_used)]
fn fixture_graph() -> graphify_build::Graph {
    let val: Value =
        serde_json::from_str(EXTRACTION_JSON).expect("fixture extraction.json must parse");
    build_from_json(val, false, None).expect("fixture JSON must build a valid graph")
}

fn fixture_communities() -> IndexMap<i64, Vec<String>> {
    cluster(&fixture_graph(), 1.0, None)
}

#[test]
fn obsidian_creates_node_notes() -> TestResult {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir()?;
    let count = to_obsidian(&g, &communities, tmp.path(), None, None)?;
    // Returned count is node_count + community_notes_written.
    assert_eq!(count, g.node_count() + communities.len());

    // At least one node note must exist (filename derived from label).
    let md_files: Vec<_> = fs::read_dir(tmp.path())?
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == std::ffi::OsStr::new("md"))
        })
        .collect();
    assert!(!md_files.is_empty(), "expected node markdown files");
    Ok(())
}

#[test]
fn obsidian_creates_community_notes() -> TestResult {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir()?;
    to_obsidian(&g, &communities, tmp.path(), None, None)?;

    let comm_notes: Vec<_> = fs::read_dir(tmp.path())?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("_COMMUNITY_"))
        .collect();
    assert_eq!(comm_notes.len(), communities.len());
    Ok(())
}

#[test]
fn obsidian_writes_graph_json_for_colors() -> TestResult {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir()?;
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    for cid in communities.keys() {
        labels.insert(*cid, format!("Community-{cid}"));
    }
    to_obsidian(&g, &communities, tmp.path(), Some(&labels), None)?;

    let obs_json = tmp.path().join(".obsidian").join("graph.json");
    assert!(obs_json.exists(), ".obsidian/graph.json missing");
    let text = fs::read_to_string(&obs_json)?;
    let parsed: Value = serde_json::from_str(&text)?;
    assert!(parsed.get("colorGroups").is_some());
    Ok(())
}

#[test]
fn obsidian_uses_cohesion_in_community_notes() -> TestResult {
    let g = fixture_graph();
    let communities = fixture_communities();
    let tmp = tempdir()?;
    let mut cohesion: IndexMap<i64, f64> = IndexMap::new();
    #[allow(clippy::cast_precision_loss)]
    for (i, cid) in communities.keys().enumerate() {
        cohesion.insert(*cid, 0.5_f64 + (i as f64) * 0.1);
    }
    to_obsidian(&g, &communities, tmp.path(), None, Some(&cohesion))?;

    let comm_notes: Vec<_> = fs::read_dir(tmp.path())?
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
    Ok(())
}

#[test]
fn obsidian_handles_duplicate_node_labels() -> TestResult {
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
    )?;
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["a1".to_string(), "a2".to_string()]);

    let tmp = tempdir()?;
    to_obsidian(&graph, &communities, tmp.path(), None, None)?;

    let names: Vec<String> = fs::read_dir(tmp.path())?
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == std::ffi::OsStr::new("md"))
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    // Should have two distinct files for "Foo" (one base + one suffixed).
    let foo_files: Vec<&String> = names.iter().filter(|n| n.starts_with("Foo")).collect();
    assert_eq!(
        foo_files.len(),
        2,
        "expected 2 distinct files for duplicate label 'Foo', got: {names:?}"
    );
    assert!(
        foo_files.iter().any(|n| n.as_str() == "Foo.md"),
        "expected one of the duplicates to keep the base 'Foo.md' (got: {names:?})"
    );
    Ok(())
}

#[test]
fn obsidian_handles_empty_communities() -> TestResult {
    let graph = build_from_json(
        json!({
            "nodes": [{"id": "x", "label": "Solo", "community": 0}],
            "edges": []
        }),
        false,
        None,
    )?;
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["x".to_string()]);

    let tmp = tempdir()?;
    let count = to_obsidian(&graph, &communities, tmp.path(), None, None)?;
    assert_eq!(count, graph.node_count() + 1);
    Ok(())
}
