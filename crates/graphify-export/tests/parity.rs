//! Parity tests for `graphify-export`.
//!
//! 1:1 ports of `graphify-py/tests/test_export.py`.

#![allow(clippy::expect_used, clippy::unwrap_used, unsafe_code)]

use graphify_build::build_from_json;
use graphify_cluster::cluster;
use graphify_export::{
    attach_hyperedges, backup_if_protected, prune_dangling_edges, to_canvas, to_cypher, to_graphml,
    to_html, to_json, to_svg,
};
use indexmap::IndexMap;
use serde_json::{Value, json};
use serial_test::serial;
use tempfile::tempdir;

// ── Fixture helpers ───────────────────────────────────────────────────────────

const EXTRACTION_JSON: &str = include_str!("../../../graphify-py/tests/fixtures/extraction.json");

fn make_graph() -> graphify_build::Graph {
    let val: Value = serde_json::from_str(EXTRACTION_JSON).unwrap();
    build_from_json(val, false, None).unwrap()
}

fn make_communities() -> IndexMap<i64, Vec<String>> {
    let g = make_graph();
    cluster(&g, 1.0, None)
}

// ── to_json ───────────────────────────────────────────────────────────────────

#[test]
fn test_to_json_creates_file() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.json");
    to_json(&g, &communities, &out, true, None).unwrap();
    assert!(out.exists());
}

#[test]
fn test_to_json_valid_json() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.json");
    to_json(&g, &communities, &out, true, None).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    assert!(data.get("nodes").is_some(), "nodes key missing");
    assert!(data.get("links").is_some(), "links key missing");
}

#[test]
fn test_to_json_nodes_have_community() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.json");
    to_json(&g, &communities, &out, true, None).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    let nodes = data["nodes"].as_array().unwrap();
    for node in nodes {
        assert!(
            node.get("community").is_some(),
            "node missing 'community' field: {node}"
        );
    }
}

// ── to_cypher ─────────────────────────────────────────────────────────────────

#[test]
fn test_to_cypher_creates_file() {
    let g = make_graph();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("cypher.txt");
    to_cypher(&g, &out).unwrap();
    assert!(out.exists());
}

#[test]
fn test_to_cypher_contains_merge_statements() {
    let g = make_graph();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("cypher.txt");
    to_cypher(&g, &out).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(
        content.contains("MERGE"),
        "Cypher output missing MERGE statements"
    );
}

// ── to_graphml ────────────────────────────────────────────────────────────────

#[test]
fn test_to_graphml_creates_file() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out).unwrap();
    assert!(out.exists());
}

#[test]
fn test_to_graphml_valid_xml() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("<graphml"), "GraphML missing <graphml");
    assert!(content.contains("<node"), "GraphML missing <node");
}

#[test]
fn test_to_graphml_has_community_attribute() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(
        content.contains("community"),
        "GraphML missing community attribute"
    );
}

// ── to_html ───────────────────────────────────────────────────────────────────

#[test]
fn test_to_html_creates_file() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).unwrap();
    assert!(out.exists());
}

#[test]
fn test_to_html_contains_visjs() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(
        content.contains("vis-network"),
        "HTML missing vis-network reference"
    );
}

#[test]
fn test_to_html_contains_search() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).unwrap();
    let content = std::fs::read_to_string(&out).unwrap().to_lowercase();
    assert!(content.contains("search"), "HTML missing 'search' element");
}

#[test]
fn test_to_html_contains_legend_with_labels() {
    let g = make_graph();
    let communities = make_communities();
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    for cid in communities.keys() {
        labels.insert(*cid, format!("Group {cid}"));
    }
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, Some(&labels), None, None).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("Group 0"), "HTML legend missing 'Group 0'");
}

#[test]
fn test_to_html_contains_nodes_and_edges() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("RAW_NODES"), "HTML missing RAW_NODES");
    assert!(content.contains("RAW_EDGES"), "HTML missing RAW_EDGES");
}

#[test]
fn test_to_html_member_counts_accepted() {
    let g = make_graph();
    let communities = make_communities();
    let member_counts: IndexMap<i64, usize> = communities
        .iter()
        .map(|(cid, members)| (*cid, members.len()))
        .collect();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, Some(&member_counts), None).unwrap();
    assert!(out.exists());
}

// ── to_canvas ─────────────────────────────────────────────────────────────────

#[test]
fn test_to_canvas_file_paths_relative_to_vault() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &communities, &out, None, None).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    let file_nodes: Vec<&Value> = data["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n.get("type").and_then(Value::as_str) == Some("file"))
        .collect();
    assert!(!file_nodes.is_empty(), "canvas should contain file nodes");
    for node in file_nodes {
        let file = node["file"].as_str().unwrap();
        assert!(
            !file.contains('/'),
            "file path should not contain '/': {file}"
        );
        assert!(
            std::path::Path::new(file)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md")),
            "file should end with .md: {file}"
        );
    }
}

// ── backup_if_protected ───────────────────────────────────────────────────────

#[test]
#[serial(backup_env)]
fn test_backup_no_graph_json() {
    let tmp = tempdir().unwrap();
    assert!(backup_if_protected(tmp.path()).is_none());
}

#[test]
#[serial(backup_env)]
fn test_backup_no_markers() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#).unwrap();
    assert!(backup_if_protected(tmp.path()).is_none());
}

#[test]
#[serial(backup_env)]
fn test_backup_semantic_marker() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#).unwrap();
    std::fs::write(tmp.path().join("GRAPH_REPORT.md"), "# Report").unwrap();
    std::fs::write(
        tmp.path().join(".graphify_semantic_marker"),
        r#"{"output_tokens": 1234}"#,
    )
    .unwrap();
    let result = backup_if_protected(tmp.path());
    assert!(result.is_some(), "expected backup to be taken");
    let backup_dir = result.unwrap();
    assert!(backup_dir.is_dir());
    assert!(backup_dir.join("graph.json").exists());
    assert!(backup_dir.join("GRAPH_REPORT.md").exists());
    assert!(backup_dir.join(".graphify_semantic_marker").exists());
}

#[test]
#[serial(backup_env)]
fn test_backup_curated_labels() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#).unwrap();
    std::fs::write(
        tmp.path().join(".graphify_labels.json"),
        r#"{"0": "Auth Pipeline", "1": "Community 1"}"#,
    )
    .unwrap();
    let result = backup_if_protected(tmp.path());
    assert!(result.is_some(), "expected backup for curated labels");
}

#[test]
#[serial(backup_env)]
fn test_backup_default_labels_only() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#).unwrap();
    std::fs::write(
        tmp.path().join(".graphify_labels.json"),
        r#"{"0": "Community 0", "1": "Community 1"}"#,
    )
    .unwrap();
    assert!(backup_if_protected(tmp.path()).is_none());
}

#[test]
#[serial(backup_env)]
fn test_backup_same_day_collision() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#).unwrap();
    std::fs::write(tmp.path().join(".graphify_semantic_marker"), "{}").unwrap();
    let b1 = backup_if_protected(tmp.path()).expect("first backup should succeed");
    let b2 = backup_if_protected(tmp.path()).expect("second backup should succeed");
    assert_ne!(b1, b2);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert_eq!(
        b2.file_name().unwrap().to_string_lossy(),
        format!("{today}_2")
    );
}

#[test]
#[serial(backup_env)]
fn test_backup_env_disable() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#).unwrap();
    std::fs::write(tmp.path().join(".graphify_semantic_marker"), "{}").unwrap();
    // SAFETY: nextest runs each test in a separate process, so env mutation is safe.
    unsafe {
        std::env::set_var("GRAPHIFY_NO_BACKUP", "1");
    }
    let result = backup_if_protected(tmp.path());
    // SAFETY: same as above.
    unsafe {
        std::env::remove_var("GRAPHIFY_NO_BACKUP");
    }
    assert!(result.is_none());
}

// ── to_svg ────────────────────────────────────────────────────────────────────

#[test]
fn test_to_svg_creates_file() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.svg");
    to_svg(&g, &communities, &out, None, (8, 6)).unwrap();
    assert!(out.exists());
}

#[test]
fn test_to_svg_valid_svg() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().unwrap();
    let out = tmp.path().join("graph.svg");
    to_svg(&g, &communities, &out, None, (8, 6)).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("<svg"), "SVG output missing <svg element");
    assert!(content.contains("</svg>"), "SVG output missing </svg>");
}

// ── prune_dangling_edges ──────────────────────────────────────────────────────

#[test]
fn test_prune_dangling_edges_removes_orphans() {
    let data = json!({
        "nodes": [{"id": "a"}, {"id": "b"}],
        "links": [
            {"source": "a", "target": "b"},
            {"source": "a", "target": "c"}
        ]
    });
    let (pruned, removed) = prune_dangling_edges(data);
    assert_eq!(removed, 1);
    let links = pruned["links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
}

#[test]
fn test_prune_dangling_edges_all_valid() {
    let data = json!({
        "nodes": [{"id": "a"}, {"id": "b"}],
        "links": [
            {"source": "a", "target": "b"}
        ]
    });
    let (pruned, removed) = prune_dangling_edges(data);
    assert_eq!(removed, 0);
    let links = pruned["links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
}

// ── attach_hyperedges ─────────────────────────────────────────────────────────

#[test]
fn test_attach_hyperedges_adds_to_graph() {
    let val: Value = serde_json::from_str(EXTRACTION_JSON).unwrap();
    let mut g = build_from_json(val, false, None).unwrap();
    let hyperedges =
        vec![json!({"id": "he1", "nodes": ["n_transformer", "n_attention"], "label": "group"})];
    attach_hyperedges(&mut g, &hyperedges);
    let stored = g.graph_attrs["hyperedges"].as_array().unwrap();
    assert_eq!(stored.len(), 1);
}

#[test]
fn test_attach_hyperedges_deduplicates() {
    let val: Value = serde_json::from_str(EXTRACTION_JSON).unwrap();
    let mut g = build_from_json(val, false, None).unwrap();
    // First attach
    let he1 = vec![json!({"id": "he1"})];
    attach_hyperedges(&mut g, &he1);
    // Second attach with duplicate + new entry
    let both = vec![json!({"id": "he1"}), json!({"id": "he2"})];
    attach_hyperedges(&mut g, &both);
    let stored = g.graph_attrs["hyperedges"].as_array().unwrap();
    assert_eq!(stored.len(), 2, "he1 should not be duplicated");
}
