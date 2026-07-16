//! Parity tests for `graphify-export`.
//!
//! 1:1 ports of `graphify-py/tests/test_export.py`.

// `.expect("...")` is the sanctioned style for `tests/parity.rs` (AGENTS.md
// permits the file-top `expect_used` allow); a setup/build failure surfaces as
// a clear test panic. Kept consistent with every other crate's `parity.rs`.
#![allow(clippy::expect_used, unsafe_code)]

use graphify_build::build_from_json;
use graphify_cluster::cluster;
use graphify_export::{
    attach_hyperedges, backup_if_protected, prune_dangling_edges, to_canvas, to_cypher, to_graphml,
    to_html, to_json, to_obsidian, to_svg,
};
use indexmap::IndexMap;
use serde_json::{Value, json};
use serial_test::serial;
use std::path::Path;
use tempfile::tempdir;

// ── Fixture helpers ───────────────────────────────────────────────────────────

const EXTRACTION_JSON: &str = include_str!("../../../graphify-py/tests/fixtures/extraction.json");

fn make_graph() -> graphify_build::Graph {
    let val: Value = serde_json::from_str(EXTRACTION_JSON).expect("valid JSON");
    build_from_json(val, false, None).expect("build_from_json")
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
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    to_json(&g, &communities, &out, true, None, None).expect("test invariant");
    assert!(out.exists());
}

#[test]
fn test_to_json_valid_json() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    to_json(&g, &communities, &out, true, None, None).expect("test invariant");
    let text = std::fs::read_to_string(&out).expect("read fixture");
    let data: Value = serde_json::from_str(&text).expect("valid JSON");
    assert!(data.get("nodes").is_some(), "nodes key missing");
    assert!(data.get("links").is_some(), "links key missing");
}

#[test]
fn test_to_json_nodes_have_community() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    to_json(&g, &communities, &out, true, None, None).expect("test invariant");
    let text = std::fs::read_to_string(&out).expect("read fixture");
    let data: Value = serde_json::from_str(&text).expect("valid JSON");
    let nodes = data["nodes"].as_array().expect("array field");
    for node in nodes {
        assert!(
            node.get("community").is_some(),
            "node missing 'community' field: {node}"
        );
    }
}

#[test]
fn test_to_json_community_name() {
    // #1305: `community_labels` writes a human `community_name` per node, with a
    // `Community {cid}` fallback for community ids that have no label. Split all
    // nodes into two communities, label only cid 0, and assert both branches.
    let g = make_graph();
    let node_ids: Vec<String> = g.nodes().map(|(id, _)| id.clone()).collect();
    assert!(
        node_ids.len() >= 2,
        "fixture must have >= 2 nodes to split into two communities"
    );
    let mid = node_ids.len() / 2;
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, node_ids[..mid].to_vec());
    communities.insert(1, node_ids[mid..].to_vec());

    let mut labels: IndexMap<i64, String> = IndexMap::new();
    labels.insert(0, "Labeled Community".to_string()); // cid 1 left unlabeled

    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    to_json(&g, &communities, &out, true, None, Some(&labels)).expect("test invariant");
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read")).expect("valid JSON");

    let mut saw_labeled = false;
    let mut saw_fallback = false;
    for node in data["nodes"].as_array().expect("array field") {
        let cid = node
            .get("community")
            .and_then(Value::as_i64)
            .expect("every node is in a community");
        let name = node
            .get("community_name")
            .and_then(Value::as_str)
            .expect("node with a community id must carry community_name");
        if cid == 0 {
            assert_eq!(name, "Labeled Community");
            saw_labeled = true;
        } else {
            assert_eq!(name, format!("Community {cid}"));
            saw_fallback = true;
        }
    }
    assert!(saw_labeled, "expected nodes in the labeled community");
    assert!(
        saw_fallback,
        "expected nodes hitting the 'Community {{cid}}' fallback"
    );

    // None / empty labels must omit `community_name` entirely (unchanged behavior).
    let empty: IndexMap<i64, String> = IndexMap::new();
    let out_empty = tmp.path().join("graph_empty.json");
    to_json(&g, &communities, &out_empty, true, None, Some(&empty)).expect("test invariant");
    let data_empty: Value =
        serde_json::from_str(&std::fs::read_to_string(&out_empty).expect("read"))
            .expect("valid JSON");
    for node in data_empty["nodes"].as_array().expect("array field") {
        assert!(
            node.get("community_name").is_none(),
            "empty labels must omit community_name: {node}"
        );
    }
}

// ── to_cypher ─────────────────────────────────────────────────────────────────

#[test]
fn test_to_cypher_creates_file() {
    let g = make_graph();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("cypher.txt");
    to_cypher(&g, &out).expect("test invariant");
    assert!(out.exists());
}

#[test]
fn test_to_cypher_contains_merge_statements() {
    let g = make_graph();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("cypher.txt");
    to_cypher(&g, &out).expect("test invariant");
    let content = std::fs::read_to_string(&out).expect("read fixture");
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
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out).expect("test invariant");
    assert!(out.exists());
}

#[test]
fn test_to_graphml_valid_xml() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out).expect("test invariant");
    let content = std::fs::read_to_string(&out).expect("read fixture");
    assert!(content.contains("<graphml"), "GraphML missing <graphml");
    assert!(content.contains("<node"), "GraphML missing <node");
}

#[test]
fn test_to_graphml_has_community_attribute() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out).expect("test invariant");
    let content = std::fs::read_to_string(&out).expect("read fixture");
    assert!(
        content.contains("community"),
        "GraphML missing community attribute"
    );
}

#[test]
fn test_to_graphml_tolerates_none_attribute_values() -> Result<(), Box<dyn std::error::Error>> {
    // A null attribute value must coerce to "" so a node/edge with a null field
    // still exports (no crash). graphify-py needs this because nx.write_graphml
    // raises ValueError on None (#1502); the hand-written Rust GraphML already
    // renders null as empty, so this pins that contract.
    let mut g = make_graph();
    let communities = make_communities();
    // Inject a null-valued attribute on one node...
    let (nid, mut nattrs) = {
        let (id, attrs) = g.nodes().next().ok_or("graph has at least one node")?;
        (id.clone(), attrs.clone())
    };
    nattrs.insert("nullable_field".to_string(), Value::Null);
    g.add_node(&nid, nattrs);
    // ...and on one edge.
    let edge_info = g
        .edges()
        .next()
        .map(|e| (e.source.clone(), e.target.clone(), e.attrs.clone()));
    if let Some((src, tgt, mut eattrs)) = edge_info {
        eattrs.insert("nullable_field".to_string(), Value::Null);
        g.add_edge(&src, &tgt, eattrs);
    }

    let tmp = tempdir()?;
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out)?;
    let content = std::fs::read_to_string(&out)?;
    assert!(content.contains("<graphml"), "GraphML missing <graphml");
    Ok(())
}

// ── to_html ───────────────────────────────────────────────────────────────────

#[test]
fn test_to_html_creates_file() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).expect("test invariant");
    assert!(out.exists());
}

#[test]
fn test_to_html_contains_visjs() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).expect("test invariant");
    let content = std::fs::read_to_string(&out).expect("read fixture");
    assert!(
        content.contains("vis-network"),
        "HTML missing vis-network reference"
    );
}

#[test]
fn test_to_html_contains_search() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).expect("test invariant");
    let content = std::fs::read_to_string(&out)
        .expect("read fixture")
        .to_lowercase();
    assert!(content.contains("search"), "HTML missing 'search' element");
}

/// Extract and parse the `const RAW_NODES = [...]` vis.js node list from the
/// interactive HTML export.
fn raw_nodes_from_html(html: &str) -> Vec<Value> {
    let after = html
        .split("const RAW_NODES = ")
        .nth(1)
        .expect("RAW_NODES marker");
    let arr = after.split(";\n").next().expect("RAW_NODES array");
    serde_json::from_str(arr).expect("RAW_NODES is valid JSON")
}

fn html_with_overlay(overlay: Value) -> Vec<Value> {
    let mut g = build_from_json(
        json!({
            "nodes": [
                {"id": "n_transformer", "label": "Transformer", "source_file": "t.py"},
                {"id": "other", "label": "Other", "source_file": "o.py"},
            ],
            "edges": [],
        }),
        false,
        None,
    )
    .expect("build graph");
    g.graph_attrs
        .insert("_learning_overlay".to_string(), overlay);
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &IndexMap::new(), &out, None, None, None).expect("to_html");
    raw_nodes_from_html(&std::fs::read_to_string(&out).expect("read html"))
}

#[test]
fn test_to_html_annotated_node_gets_learning_status_and_ring() {
    // #1441: a preferred node gets learning_status/stale fields, a green ring
    // border, borderWidth 3, and a Lesson tooltip; an un-annotated node is untouched.
    let nodes = html_with_overlay(json!({
        "n_transformer": {"status": "preferred", "uses": 3, "score": 2.4, "stale": false}
    }));
    let ann = nodes
        .iter()
        .find(|n| n["id"] == "n_transformer")
        .expect("annotated node");
    assert_eq!(ann["learning_status"], "preferred");
    assert_eq!(ann["learning_stale"], false);
    assert_eq!(ann["color"]["border"], "#22c55e");
    assert_eq!(ann["borderWidth"], 3);
    assert!(
        ann["title"]
            .as_str()
            .expect("title")
            .contains("Lesson: preferred source")
    );
    let other = nodes
        .iter()
        .find(|n| n["id"] == "other")
        .expect("other node");
    assert!(other.get("learning_status").is_none());
    assert!(other.get("learning_stale").is_none());
}

#[test]
fn test_to_html_contested_stale_node_gets_dashed_desaturated_ring() {
    let nodes = html_with_overlay(json!({
        "n_transformer": {"status": "contested", "uses": 2, "neg": 3, "score": -0.5, "stale": true}
    }));
    let ann = nodes
        .iter()
        .find(|n| n["id"] == "n_transformer")
        .expect("annotated node");
    assert_eq!(ann["learning_status"], "contested");
    assert_eq!(ann["learning_stale"], true);
    assert_eq!(ann["color"]["border"], "#9ca3af");
    assert_eq!(ann["shapeProperties"]["borderDashes"], json!([4, 4]));
    assert!(
        ann["title"]
            .as_str()
            .expect("title")
            .contains("code changed")
    );
}

#[test]
fn test_to_html_unannotated_identical_to_pre_feature() {
    // #1441: with no overlay, HTML is byte-identical whether `_learning_overlay`
    // is omitted or an empty object — no learning fields leak into the render.
    let g = make_graph();
    let communities = make_communities();
    let mut g_empty = make_graph();
    g_empty
        .graph_attrs
        .insert("_learning_overlay".to_string(), json!({}));
    let tmp = tempdir().expect("tempdir");
    let a = tmp.path().join("a.html");
    let b = tmp.path().join("b.html");
    to_html(&g, &communities, &a, None, None, None).expect("to_html a");
    to_html(&g_empty, &communities, &b, None, None, None).expect("to_html b");
    // The output path appears in the title, so compare with the name normalized.
    let ca = std::fs::read_to_string(&a)
        .expect("read a")
        .replace("a.html", "X.html");
    let cb = std::fs::read_to_string(&b)
        .expect("read b")
        .replace("b.html", "X.html");
    assert_eq!(ca, cb);
    assert!(!ca.contains("learning_status"));
}

#[test]
fn test_to_html_contains_legend_with_labels() {
    let g = make_graph();
    let communities = make_communities();
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    for cid in communities.keys() {
        labels.insert(*cid, format!("Group {cid}"));
    }
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, Some(&labels), None, None).expect("test invariant");
    let content = std::fs::read_to_string(&out).expect("read fixture");
    assert!(content.contains("Group 0"), "HTML legend missing 'Group 0'");
}

#[test]
fn test_to_html_contains_nodes_and_edges() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).expect("test invariant");
    let content = std::fs::read_to_string(&out).expect("read fixture");
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
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, Some(&member_counts), None).expect("test invariant");
    assert!(out.exists());
}

// ── to_canvas ─────────────────────────────────────────────────────────────────

#[test]
fn test_to_canvas_file_paths_relative_to_vault() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &communities, &out, None, None).expect("test invariant");
    let text = std::fs::read_to_string(&out).expect("read fixture");
    let data: Value = serde_json::from_str(&text).expect("valid JSON");
    let file_nodes: Vec<&Value> = data["nodes"]
        .as_array()
        .expect("test invariant")
        .iter()
        .filter(|n| n.get("type").and_then(Value::as_str) == Some("file"))
        .collect();
    assert!(!file_nodes.is_empty(), "canvas should contain file nodes");
    for node in file_nodes {
        let file = node["file"].as_str().expect("string field");
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

#[test]
fn test_to_canvas_no_communities_still_populates() {
    // #1324: empty communities (e.g. --no-cluster builds) on a populated graph
    // must NOT produce the 32-byte empty `{"nodes": [], "edges": []}` shell.
    let g = make_graph();
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.canvas");
    // no community data — the bug condition
    to_canvas(&g, &communities, &out, None, None).expect("test invariant");
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read")).expect("valid JSON");
    let nodes = data["nodes"].as_array().expect("array field");
    let edges = data["edges"].as_array().expect("array field");
    assert!(
        nodes.len() >= g.node_count(),
        "canvas should hold at least one entry per graph node"
    );
    assert!(!edges.is_empty(), "canvas should hold at least one edge");
    let size = std::fs::metadata(&out).expect("metadata").len();
    assert!(
        size > 32,
        "canvas must not be the empty 32-byte shell (got {size})"
    );
}

#[test]
fn test_to_canvas_all_dangling_communities_synthesize_real_cards() {
    // A non-empty community whose members ALL dangle (stale index / merge artifact)
    // must not leave an empty group box; the graph's real nodes are synthesized
    // into a single all-nodes community instead (#1324 applied to the filtered map).
    let g = make_graph();
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(7, vec!["ghost_a".to_string(), "ghost_b".to_string()]);
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &communities, &out, None, None).expect("test invariant");
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read")).expect("valid JSON");
    let nodes = data["nodes"].as_array().expect("array field");
    let file_cards = nodes.iter().filter(|n| n["type"] == "file").count();
    assert_eq!(
        file_cards,
        g.node_count(),
        "each real node appears once as a card (no duplication): {file_cards} vs {}",
        g.node_count()
    );
    let group_count = nodes.iter().filter(|n| n["type"] == "group").count();
    assert_eq!(
        group_count, 1,
        "only the synthesized all-nodes group should remain, not the dangling one"
    );
}

#[test]
fn test_to_canvas_fallback_group_ignores_stale_community_0_label() {
    // #1324: the synthesized all-nodes fallback keys on a sentinel id, so a stale
    // `community_labels` entry for community 0 never leaks into its group label.
    let g = make_graph();
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new(); // triggers the fallback
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    labels.insert(0, "AuthModule".to_string());
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &communities, &out, Some(&labels), None).expect("test invariant");
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read")).expect("valid JSON");
    let nodes = data["nodes"].as_array().expect("array field");
    let group = nodes
        .iter()
        .find(|n| n["type"] == "group")
        .expect("a synthesized group");
    assert_eq!(
        group["label"].as_str(),
        Some("Community 0"),
        "the fallback group must not inherit the stale community-0 label"
    );
}

// ── backup_if_protected ───────────────────────────────────────────────────────

#[test]
#[serial(backup_env)]
fn test_backup_no_graph_json() {
    let tmp = tempdir().expect("tempdir");
    assert!(backup_if_protected(tmp.path()).is_none());
}

#[test]
#[serial(backup_env)]
fn test_backup_no_markers() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#)
        .expect("test invariant");
    assert!(backup_if_protected(tmp.path()).is_none());
}

#[test]
#[serial(backup_env)]
fn test_backup_semantic_marker() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#)
        .expect("test invariant");
    std::fs::write(tmp.path().join("GRAPH_REPORT.md"), "# Report").expect("test invariant");
    std::fs::write(
        tmp.path().join(".graphify_semantic_marker"),
        r#"{"output_tokens": 1234}"#,
    )
    .expect("test invariant");
    let result = backup_if_protected(tmp.path());
    assert!(result.is_some(), "expected backup to be taken");
    let backup_dir = result.expect("test invariant");
    assert!(backup_dir.is_dir());
    assert!(backup_dir.join("graph.json").exists());
    assert!(backup_dir.join("GRAPH_REPORT.md").exists());
    assert!(backup_dir.join(".graphify_semantic_marker").exists());
}

#[test]
#[serial(backup_env)]
fn test_backup_curated_labels() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#)
        .expect("test invariant");
    std::fs::write(
        tmp.path().join(".graphify_labels.json"),
        r#"{"0": "Auth Pipeline", "1": "Community 1"}"#,
    )
    .expect("test invariant");
    let result = backup_if_protected(tmp.path());
    assert!(result.is_some(), "expected backup for curated labels");
}

#[test]
#[serial(backup_env)]
fn test_backup_default_labels_only() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#)
        .expect("test invariant");
    std::fs::write(
        tmp.path().join(".graphify_labels.json"),
        r#"{"0": "Community 0", "1": "Community 1"}"#,
    )
    .expect("test invariant");
    assert!(backup_if_protected(tmp.path()).is_none());
}

#[test]
#[serial(backup_env)]
fn test_backup_same_day_no_accumulation() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#)
        .expect("test invariant");
    std::fs::write(tmp.path().join(".graphify_semantic_marker"), "{}").expect("test invariant");
    let b1 = backup_if_protected(tmp.path()).expect("first backup should succeed");
    let b2 = backup_if_protected(tmp.path()).expect("second backup should succeed");
    // Same content on same day → reuse existing folder, no _2 suffix.
    assert_eq!(b1, b2);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert_eq!(
        b1.file_name().expect("has filename").to_string_lossy(),
        today
    );
}

#[test]
#[serial(backup_env)]
fn test_backup_same_day_changed_content() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#)
        .expect("test invariant");
    std::fs::write(tmp.path().join(".graphify_semantic_marker"), "{}").expect("test invariant");
    let b1 = backup_if_protected(tmp.path()).expect("first backup should succeed");
    // Change graph.json content, then re-back up — the existing folder is
    // overwritten in place; we still get one folder per day.
    std::fs::write(
        tmp.path().join("graph.json"),
        r#"{"nodes":[{"id":"x"}],"links":[]}"#,
    )
    .expect("test invariant");
    let b2 = backup_if_protected(tmp.path()).expect("second backup should succeed");
    assert_eq!(b1, b2);
    assert_eq!(
        std::fs::read_to_string(b2.join("graph.json")).expect("read backed-up graph.json"),
        r#"{"nodes":[{"id":"x"}],"links":[]}"#
    );
}

#[test]
#[serial(backup_env)]
fn test_backup_env_disable() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("graph.json"), r#"{"nodes":[],"links":[]}"#)
        .expect("test invariant");
    std::fs::write(tmp.path().join(".graphify_semantic_marker"), "{}").expect("test invariant");
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
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.svg");
    to_svg(&g, &communities, &out, None, (8, 6)).expect("test invariant");
    assert!(out.exists());
}

#[test]
fn test_to_svg_valid_svg() {
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.svg");
    to_svg(&g, &communities, &out, None, (8, 6)).expect("test invariant");
    let content = std::fs::read_to_string(&out).expect("read fixture");
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
    let links = pruned["links"].as_array().expect("array field");
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
    let links = pruned["links"].as_array().expect("array field");
    assert_eq!(links.len(), 1);
}

// ── attach_hyperedges ─────────────────────────────────────────────────────────

#[test]
fn test_attach_hyperedges_adds_to_graph() {
    let val: Value = serde_json::from_str(EXTRACTION_JSON).expect("valid JSON");
    let mut g = build_from_json(val, false, None).expect("build_from_json");
    let hyperedges =
        vec![json!({"id": "he1", "nodes": ["n_transformer", "n_attention"], "label": "group"})];
    attach_hyperedges(&mut g, &hyperedges);
    let stored = g.graph_attrs["hyperedges"].as_array().expect("array field");
    assert_eq!(stored.len(), 1);
}

#[test]
fn test_attach_hyperedges_deduplicates() {
    let val: Value = serde_json::from_str(EXTRACTION_JSON).expect("valid JSON");
    let mut g = build_from_json(val, false, None).expect("build_from_json");
    // First attach
    let he1 = vec![json!({"id": "he1"})];
    attach_hyperedges(&mut g, &he1);
    // Second attach with duplicate + new entry
    let both = vec![json!({"id": "he1"}), json!({"id": "he2"})];
    attach_hyperedges(&mut g, &both);
    let stored = g.graph_attrs["hyperedges"].as_array().expect("array field");
    assert_eq!(stored.len(), 2, "he1 should not be duplicated");
}

#[test]
fn test_attach_hyperedges_deduplicates_within_one_batch() {
    // graphify-py records each accepted id into `seen_ids` as it appends, so a
    // duplicate id in the SAME batch is dropped too — not only across calls.
    let val: Value = serde_json::from_str(EXTRACTION_JSON).expect("valid JSON");
    let mut g = build_from_json(val, false, None).expect("build_from_json");
    let batch = vec![
        json!({"id": "dup", "label": "first"}),
        json!({"id": "dup", "label": "second"}),
        json!({"id": "unique", "label": "third"}),
    ];
    attach_hyperedges(&mut g, &batch);
    let stored = g.graph_attrs["hyperedges"].as_array().expect("array field");
    assert_eq!(
        stored.len(),
        2,
        "the duplicate `dup` within one batch must collapse to a single entry"
    );
    // The first occurrence wins (append-then-record order).
    let dup = stored
        .iter()
        .find(|h| h.get("id").and_then(Value::as_str) == Some("dup"))
        .expect("dup present");
    assert_eq!(dup.get("label").and_then(Value::as_str), Some("first"));
}

#[test]
fn test_to_html_aggregated_remaps_hyperedges_to_communities() {
    // graphify-py #1006: when the graph exceeds the viz limit and is aggregated
    // into a community meta-graph, hyperedges must be remapped from semantic
    // node IDs to community IDs. Cross-community hyperedges survive (label
    // derived from the relation); hyperedges within a single community collapse
    // and are dropped.
    let extraction = json!({
        "nodes": [
            {"id": "n1", "label": "N1"},
            {"id": "n2", "label": "N2"},
            {"id": "n3", "label": "N3"},
            {"id": "n4", "label": "N4"},
        ],
        "edges": [],
    });
    let mut g = build_from_json(extraction, false, None).expect("build_from_json");
    attach_hyperedges(
        &mut g,
        &[
            // Spans community 0 and community 1 → kept.
            json!({"id": "h1", "relation": "shares_data_flow", "nodes": ["n1", "n3"]}),
            // Entirely within community 0 → fewer than 2 communities → dropped.
            json!({"id": "h2", "relation": "intra_only", "nodes": ["n1", "n2"]}),
        ],
    );

    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["n1".to_string(), "n2".to_string()]);
    communities.insert(1, vec!["n3".to_string(), "n4".to_string()]);

    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    // node_limit = Some(2): 4 nodes > 2 → aggregated community view.
    to_html(&g, &communities, &out, None, None, Some(2)).expect("to_html");

    let html = std::fs::read_to_string(&out).expect("read html");
    assert!(
        html.contains("shares data flow"),
        "cross-community hyperedge label should be remapped and rendered"
    );
    assert!(
        !html.contains("intra only"),
        "single-community hyperedge should be dropped from the aggregated view"
    );
}

// ── #1409: punctuation-only Obsidian/Canvas filenames ─────────────────────────

/// A 2-node graph where one node's label is all-punctuation (e.g. a `@/*`
/// tsconfig paths key) and the other is a normal symbol.
fn punct_graph(label: &str) -> graphify_build::Graph {
    let val = json!({
        "nodes": [
            {"id": "n1", "label": label, "file_type": "code", "source_file": "tsconfig.json"},
            {"id": "n2", "label": "AuthHandler", "file_type": "code", "source_file": "auth.ts"},
        ],
        "edges": [],
    });
    build_from_json(val, false, None).expect("build_from_json")
}

/// Recursively collect the file stems of every `*.md` under `dir`.
fn collect_md_stems(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_md_stems(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("md")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            out.push(stem.to_string());
        }
    }
    out
}

fn has_word_char(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric() || c == '_')
}

#[test]
fn to_obsidian_never_emits_punctuation_only_filenames() {
    // An all-punctuation label (e.g. `@/*`) must not produce a `@.md`-style
    // filename; it falls back to `unnamed` (#1409).
    let g = punct_graph("@/*");
    let communities = cluster(&g, 1.0, None);
    let tmp = tempdir().expect("tempdir");
    let written = to_obsidian(&g, &communities, tmp.path(), None, None).expect("to_obsidian");
    assert!(written > 0, "to_obsidian wrote no notes");
    let stems = collect_md_stems(tmp.path());
    assert!(!stems.is_empty(), "to_obsidian wrote no notes");
    let bad: Vec<&String> = stems.iter().filter(|s| !has_word_char(s)).collect();
    assert!(
        bad.is_empty(),
        "punctuation-only filenames emitted: {bad:?}"
    );
    assert!(
        stems
            .iter()
            .any(|s| s == "unnamed" || s.starts_with("unnamed")),
        "{stems:?}"
    );
}

#[test]
fn to_canvas_never_emits_punctuation_only_filenames() {
    // Same guard on the canvas exporter's file-node names (#1409).
    let g = punct_graph("@");
    let communities = cluster(&g, 1.0, None);
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &communities, &out, None, None).expect("to_canvas");
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read canvas")).expect("json");
    let nodes = data
        .get("nodes")
        .and_then(Value::as_array)
        .expect("nodes array");
    let file_nodes: Vec<&Value> = nodes
        .iter()
        .filter(|n| n.get("type").and_then(Value::as_str) == Some("file"))
        .collect();
    assert!(!file_nodes.is_empty(), "canvas has no file nodes");
    for n in &file_nodes {
        let file = n.get("file").and_then(Value::as_str).expect("file field");
        let stem = Path::new(file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        assert!(
            has_word_char(stem),
            "punctuation-only canvas filename: {file}"
        );
    }
}
// ── to_json anti-shrink guard (#479 / d2d1f68 fail-safe) ────────────────────

/// Build an undirected graph with `n` nodes labelled `n0..n{n-1}` (mirrors the
/// Python `_mkG` helper).
fn make_graph_n(n: usize) -> graphify_build::Graph {
    let mut g = graphify_build::Graph::new(graphify_build::GraphKind::Graph);
    for i in 0..n {
        let mut attrs = IndexMap::new();
        attrs.insert("label".to_string(), json!(format!("n{i}")));
        g.add_node(&format!("n{i}"), attrs);
    }
    g
}

#[test]
fn test_to_json_refuses_shrink() {
    // #479: refuse to silently overwrite an existing graph with fewer nodes.
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    std::fs::write(
        &out,
        r#"{"nodes":[{"id":"n0"},{"id":"n1"},{"id":"n2"},{"id":"n3"},{"id":"n4"}],"links":[]}"#,
    )
    .expect("write existing");
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    assert!(
        !to_json(&make_graph_n(2), &communities, &out, false, None, None).expect("to_json"),
        "a smaller graph must be refused when force=false"
    );
    assert!(
        to_json(&make_graph_n(2), &communities, &out, true, None, None).expect("to_json"),
        "force=true overrides the shrink guard"
    );
}

#[test]
fn test_to_json_refuses_unreadable_existing() {
    // An existing path that cannot be read as a file (here a directory) must fail
    // SAFE — refuse the overwrite rather than treat it as empty. Fail-open would be
    // the silent data-loss path #479 guards against.
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    std::fs::create_dir(&out).expect("mkdir");
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    assert!(
        !to_json(&make_graph_n(5), &communities, &out, false, None, None).expect("to_json"),
        "an unreadable existing path must be refused when force=false"
    );
}

#[test]
fn test_to_json_fails_safe_on_corrupt_existing() {
    // A non-empty but unparseable existing graph.json (corrupt or mid-write) must
    // NOT be silently overwritten — we can't verify the new graph isn't a partial
    // shrink, so fail safe (refuse) unless force is given.
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    std::fs::write(&out, "{ this has content but is not valid json").expect("write existing");
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    assert!(
        !to_json(&make_graph_n(10), &communities, &out, false, None, None).expect("to_json"),
        "an unparseable existing graph must be refused when force=false"
    );
    assert!(
        to_json(&make_graph_n(10), &communities, &out, true, None, None).expect("to_json"),
        "force=true overrides"
    );
}

#[test]
fn test_to_json_proceeds_on_empty_existing() {
    // An empty/whitespace existing file has no nodes to lose, so it is not a
    // shrink risk — the write proceeds.
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.json");
    std::fs::write(&out, "").expect("write existing");
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    assert!(
        to_json(&make_graph_n(3), &communities, &out, false, None, None).expect("to_json"),
        "an empty existing file is not a shrink — proceed"
    );
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read")).expect("valid JSON");
    assert_eq!(data["nodes"].as_array().expect("array").len(), 3);
}

// ── U8: html XSS guard + graphml non-scalar tolerance (#1838, #1831, #1775) ────

#[test]
fn test_to_html_neighbor_links_have_no_inline_onclick_xss() {
    // #1838: neighbor links must not drop an unescaped JSON.stringify(nid) into a
    // quoted inline onclick (broke every link + stored XSS from a hostile id).
    // The id is carried in an escaped data attribute and dispatched via one
    // delegated listener.
    let g = make_graph();
    let communities = make_communities();
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).expect("to_html");
    let html = std::fs::read_to_string(&out).expect("read html");
    assert!(
        !html.contains("onclick=\"focusNode("),
        "vulnerable inline onclick still present"
    );
    assert!(
        !html.contains("JSON.stringify(nid)"),
        "unescaped stringify still present"
    );
    assert!(
        html.contains("data-nid=\"${esc(nid)}\""),
        "escaped data-nid attribute missing"
    );
    assert!(
        html.contains("closest('.neighbor-link')"),
        "delegated listener missing"
    );
}

#[test]
fn test_to_html_handles_null_source_file_and_label() {
    // #1775: a node with source_file=None or label=None must not crash to_html
    // (synthetic/aggregate nodes legitimately carry null source_file).
    let mut g = graphify_build::Graph::new(graphify_build::GraphKind::Graph);
    let mut n1 = IndexMap::new();
    n1.insert("label".to_string(), json!("Foo"));
    n1.insert("source_file".to_string(), Value::Null);
    n1.insert("community".to_string(), json!(0));
    g.add_node("n1", n1);
    let mut n2 = IndexMap::new();
    n2.insert("label".to_string(), Value::Null);
    n2.insert("source_file".to_string(), json!("a.py"));
    n2.insert("community".to_string(), json!(0));
    g.add_node("n2", n2);
    let mut n3 = IndexMap::new();
    n3.insert("label".to_string(), Value::Null);
    n3.insert("source_file".to_string(), Value::Null);
    n3.insert("community".to_string(), json!(0));
    g.add_node("n3", n3);

    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(
        0,
        vec!["n1".to_string(), "n2".to_string(), "n3".to_string()],
    );
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None)
        .expect("to_html must not panic on null attrs");
    let meta = std::fs::metadata(&out).expect("stat html");
    assert!(
        out.exists() && meta.len() > 0,
        "html output must be non-empty"
    );
}

#[test]
fn test_to_graphml_tolerates_dict_and_list_attribute_values()
-> Result<(), Box<dyn std::error::Error>> {
    // #1831: a dict/list attribute (per-node metadata, tags, the graph-level
    // hyperedges list) must not crash the export — each is JSON-serialized into
    // a string-typed <data> across graph/node/edge scopes.
    let mut g = make_graph();
    let communities = make_communities();
    let (nid, mut nattrs) = {
        let (id, attrs) = g.nodes().next().ok_or("graph has at least one node")?;
        (id.clone(), attrs.clone())
    };
    nattrs.insert("metadata".to_string(), json!({"kind": "file", "size": 12}));
    nattrs.insert("tags".to_string(), json!(["x", "y"]));
    g.add_node(&nid, nattrs);
    let edge_info = g
        .edges()
        .next()
        .map(|e| (e.source.clone(), e.target.clone(), e.attrs.clone()));
    if let Some((src, tgt, mut eattrs)) = edge_info {
        eattrs.insert("ctx".to_string(), json!({"k": "v"}));
        g.add_edge(&src, &tgt, eattrs);
    }
    g.graph_attrs.insert(
        "hyperedges".to_string(),
        json!([{"nodes": [nid], "label": "h"}]),
    );
    // An internal `_`-prefixed graph attr must be dropped from the export,
    // consistent with node/edge scope (not leaked as a GraphML key).
    g.graph_attrs
        .insert("_internal".to_string(), json!("secret"));

    let tmp = tempdir()?;
    let out = tmp.path().join("graph.graphml");
    to_graphml(&g, &communities, &out)?; // must not raise
    let content = std::fs::read_to_string(&out)?;
    // The dict/list are JSON-serialized then XML-escaped (`"` -> `&quot;`).
    assert!(
        content.contains("&quot;kind&quot;:&quot;file&quot;"),
        "node dict attr not serialized"
    );
    assert!(
        content.contains("&quot;size&quot;:12"),
        "node dict numeric member missing"
    );
    assert!(
        content.contains("[&quot;x&quot;,&quot;y&quot;]"),
        "node list attr not serialized"
    );
    assert!(
        content.contains("&quot;k&quot;:&quot;v&quot;"),
        "edge dict attr not serialized"
    );
    assert!(
        !content.contains("attr.name=\"_internal\""),
        "internal `_`-prefixed graph attr leaked into GraphML"
    );
    // Graph-level hyperedges list is emitted as a graph-scoped <data> (#1831).
    // The key id is opaque (networkx-style d0/d1/…), so assert on the key
    // declaration's scope/name/type and on the serialized member content.
    assert!(
        content.contains("for=\"graph\" attr.name=\"hyperedges\" attr.type=\"string\""),
        "graph-level hyperedges key declaration missing"
    );
    assert!(
        content.contains("&quot;label&quot;:&quot;h&quot;"),
        "graph-level hyperedges data missing"
    );
    // Atomic write leaves no stale sibling temp (#1831).
    assert!(
        !out.with_extension("graphml.tmp").exists(),
        "stale .tmp left behind"
    );
    Ok(())
}

#[test]
fn test_to_graphml_preserves_native_scalar_types() -> Result<(), Box<dyn std::error::Error>> {
    // #1831: GraphML-native scalars (int/float/bool/str) keep their type so a
    // consumer reads them back native — the key is typed by the value category,
    // only non-scalars are stringified.
    let mut g = graphify_build::Graph::new(graphify_build::GraphKind::Graph);
    let mut a = IndexMap::new();
    a.insert("count".to_string(), json!(3));
    a.insert("ratio".to_string(), json!(0.5));
    a.insert("flag".to_string(), json!(true));
    a.insert("name".to_string(), json!("x"));
    g.add_node("a", a);
    g.add_node("b", IndexMap::new());
    g.add_edge("a", "b", IndexMap::new());

    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec!["a".to_string(), "b".to_string()]);
    let tmp = tempdir()?;
    let out = tmp.path().join("g.graphml");
    to_graphml(&g, &communities, &out)?;
    let content = std::fs::read_to_string(&out)?;
    // Key declarations carry the inferred native type.
    assert!(
        content.contains("attr.name=\"count\" attr.type=\"long\""),
        "int key not typed long"
    );
    assert!(
        content.contains("attr.name=\"ratio\" attr.type=\"double\""),
        "float key not typed double"
    );
    assert!(
        content.contains("attr.name=\"flag\" attr.type=\"boolean\""),
        "bool key not typed boolean"
    );
    assert!(
        content.contains("attr.name=\"name\" attr.type=\"string\""),
        "str key not typed string"
    );
    // And the values round-trip unchanged.
    assert!(content.contains(">3</data>"), "int value missing");
    assert!(content.contains(">0.5</data>"), "float value missing");
    assert!(content.contains(">true</data>"), "bool value missing");
    assert!(content.contains(">x</data>"), "str value missing");
    Ok(())
}

#[test]
fn test_to_graphml_splits_mixed_type_attr_into_typed_keys() -> Result<(), Box<dyn std::error::Error>>
{
    // #1831: when the same attribute name carries different scalar types across
    // nodes, each (name, type) gets its own <key> and each node's <data> must
    // reference the key whose declared type matches its value (networkx-style
    // split). Distinct-name tests would still pass if the split were broken.
    let mut g = graphify_build::Graph::new(graphify_build::GraphKind::Graph);
    let mut ni = IndexMap::new();
    ni.insert("val".to_string(), json!(3));
    g.add_node("ni", ni);
    let mut ns = IndexMap::new();
    ns.insert("val".to_string(), json!("hi"));
    g.add_node("ns", ns);
    let mut nb = IndexMap::new();
    nb.insert("val".to_string(), json!(true));
    g.add_node("nb", nb);

    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(
        0,
        vec!["ni".to_string(), "ns".to_string(), "nb".to_string()],
    );
    let tmp = tempdir()?;
    let out = tmp.path().join("g.graphml");
    to_graphml(&g, &communities, &out)?;
    let content = std::fs::read_to_string(&out)?;

    // Extract the XML attribute `attr="..."` value from a `<key>` line.
    let attr = |line: &str, name: &str| -> String {
        line.split(&format!("{name}=\""))
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("attribute present")
            .to_string()
    };
    // Map declared type -> key id for the `val` attribute's declarations.
    let mut id_for_type: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for line in content.lines() {
        if line.contains("<key ") && line.contains("attr.name=\"val\"") {
            id_for_type.insert(attr(line, "attr.type"), attr(line, "id"));
        }
    }
    assert_eq!(
        id_for_type.len(),
        3,
        "expected 3 typed keys for `val`, got {id_for_type:?}"
    );
    // Each node's <data> references the key whose type matches its value.
    assert!(content.contains(&format!("<data key=\"{}\">3</data>", id_for_type["long"])));
    assert!(content.contains(&format!(
        "<data key=\"{}\">hi</data>",
        id_for_type["string"]
    )));
    assert!(content.contains(&format!(
        "<data key=\"{}\">true</data>",
        id_for_type["boolean"]
    )));
    Ok(())
}

#[test]
fn test_to_html_escapes_hostile_node_id_and_label() {
    // #1838: a node id/label from a document or a scraped `graphify add` URL can
    // carry a double-quote or a `</script>` sequence. It must be embedded
    // JSON-safely so it cannot break out of the data island and inject a live
    // event handler into the locally-opened report (stored XSS).
    let mut g = graphify_build::Graph::new(graphify_build::GraphKind::Graph);
    let mut a = IndexMap::new();
    a.insert("label".to_string(), json!(r#"evil" onmouseover="alert(1)"#));
    a.insert("community".to_string(), json!(0));
    let hostile_id = r#"n"</script><img src=x onerror=alert(1)>"#;
    g.add_node(hostile_id, a);
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(0, vec![hostile_id.to_string()]);
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.html");
    to_html(&g, &communities, &out, None, None, None).expect("to_html");
    let html = std::fs::read_to_string(&out).expect("read html");
    // The `</script>` in the id is neutralized (`<\/`) in the embedded data.
    assert!(
        !html.contains("</script><img"),
        "raw </script> breakout in embedded data"
    );
    // The label's double-quote is JSON-escaped, not a raw attribute breakout.
    assert!(
        html.contains(r#"onmouseover=\"alert(1)"#),
        "hostile label not JSON-escaped"
    );
    // The fix carries the id in an escaped data attribute, never an inline onclick.
    assert!(html.contains("data-nid=\"${esc(nid)}\""));
    assert!(!html.contains("onclick=\"focusNode("));
}
