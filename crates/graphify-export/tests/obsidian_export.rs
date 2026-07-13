//! Coverage tests for `to_obsidian`.

use std::fs;

use graphify_build::build_from_json;
use graphify_cluster::cluster;
use graphify_export::{to_canvas, to_obsidian};
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

// ── dangling community members (#1236) ──────────────────────────────────────

#[test]
fn obsidian_skips_dangling_community_members() -> TestResult {
    // A community member with no backing node must be skipped, not crash the
    // export or inflate the member count / Members list.
    let graph = build_from_json(
        json!({
            "nodes": [
                {"id": "n0", "label": "Alpha", "file_type": "code", "source_file": "a.py"},
                {"id": "n1", "label": "Beta", "file_type": "code", "source_file": "b.py"},
            ],
            "edges": [
                {"source": "n0", "target": "n1", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        }),
        false,
        None,
    )?;
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    // 'agents_doc' is a synthesized member id with no backing node.
    communities.insert(
        0,
        vec!["n0".to_string(), "n1".to_string(), "agents_doc".to_string()],
    );

    let tmp = tempdir()?;
    let n = to_obsidian(&graph, &communities, tmp.path(), None, None)?;
    assert!(n > 0);

    let comm_notes: Vec<_> = fs::read_dir(tmp.path())?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("_COMMUNITY_"))
        .collect();
    assert_eq!(comm_notes.len(), 1);
    let body = fs::read_to_string(comm_notes[0].path())?;

    // Real members appear; the dangling id does not; count reflects only real ones.
    assert!(body.contains("[[Alpha]]"));
    assert!(body.contains("[[Beta]]"));
    assert!(!body.contains("agents_doc"));
    assert!(body.contains("**Members:** 2 nodes"));
    Ok(())
}

#[test]
fn obsidian_community_of_only_dangling_members_does_not_crash() -> TestResult {
    let graph = build_from_json(
        json!({
            "nodes": [{"id": "n0", "label": "Alpha", "file_type": "code", "source_file": "a.py"}],
            "edges": []
        }),
        false,
        None,
    )?;
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["n0".to_string()]);
    communities.insert(1, vec!["ghost_a".to_string(), "ghost_b".to_string()]);

    let tmp = tempdir()?;
    let n = to_obsidian(&graph, &communities, tmp.path(), None, None)?;
    assert!(n > 0);

    // The all-dangling community note still exists with a zero member count.
    let ghost = fs::read_dir(tmp.path())?
        .filter_map(Result::ok)
        .find(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("_COMMUNITY_") && name.contains("Community 1")
        })
        .map(|e| fs::read_to_string(e.path()))
        .transpose()?;
    let ghost = ghost.ok_or("expected ghost community note to exist")?;
    assert!(ghost.contains("**Members:** 0 nodes"));
    Ok(())
}

#[test]
fn canvas_dangling_community_member_does_not_crash() -> TestResult {
    // #1236 follow-up: the guard landed in to_obsidian but not to_canvas, so
    // `graphify export obsidian` (which also writes graph.canvas) still emitted
    // a spurious card / over-sized box for a dangling member. Real members get
    // cards; the dangling id does not.
    let graph = build_from_json(
        json!({
            "nodes": [
                {"id": "n0", "label": "Alpha", "file_type": "code", "source_file": "a.py"},
                {"id": "n1", "label": "Beta", "file_type": "code", "source_file": "b.py"},
            ],
            "edges": [
                {"source": "n0", "target": "n1", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        }),
        false,
        None,
    )?;
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(
        0,
        vec!["n0".to_string(), "n1".to_string(), "agents_doc".to_string()],
    );

    let tmp = tempdir()?;
    let out = tmp.path().join("graph.canvas");
    to_canvas(&graph, &communities, &out, None, None)?;
    assert!(out.exists());

    let canvas: Value = serde_json::from_str(&fs::read_to_string(&out)?)?;
    let node_ids: std::collections::HashSet<&str> = canvas["nodes"]
        .as_array()
        .ok_or("nodes array")?
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    assert!(node_ids.contains("n_n0") && node_ids.contains("n_n1"));
    assert!(!node_ids.contains("n_agents_doc"));
    Ok(())
}
