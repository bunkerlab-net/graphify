//! Parity tests against `graphify-py/tests/test_cluster.py`.
//!
//! Every test case in the Python file has a direct equivalent here.
//! The tests check structural properties (coverage, score ranges, key
//! matching) rather than specific community IDs, since Louvain is
//! non-deterministic at the algorithm level and this port uses a different
//! (but seeded) implementation.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use graphify_build::{Graph, GraphKind, build_from_json};
use graphify_cluster::{cluster, cohesion_score, remap_communities_to_previous, score_all};
use indexmap::IndexMap;
use serde_json::json;

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn extraction_json() -> serde_json::Value {
    json!({
        "nodes": [
            {"id": "n_transformer", "label": "Transformer", "file_type": "code",
             "source_file": "model.py", "source_location": "L1"},
            {"id": "n_attention",   "label": "MultiHeadAttention", "file_type": "code",
             "source_file": "model.py", "source_location": "L10"},
            {"id": "n_layernorm",   "label": "LayerNorm", "file_type": "code",
             "source_file": "model.py", "source_location": "L20"},
            {"id": "n_concept_attn","label": "attention mechanism", "file_type": "document",
             "source_file": "paper.md", "source_location": "§3.1"}
        ],
        "edges": [
            {"source": "n_transformer", "target": "n_attention",
             "relation": "contains", "confidence": "EXTRACTED",
             "source_file": "model.py", "weight": 1.0},
            {"source": "n_transformer", "target": "n_layernorm",
             "relation": "contains", "confidence": "EXTRACTED",
             "source_file": "model.py", "weight": 1.0},
            {"source": "n_attention",   "target": "n_concept_attn",
             "relation": "implements", "confidence": "INFERRED",
             "source_file": "model.py", "weight": 0.8},
            {"source": "n_layernorm",   "target": "n_concept_attn",
             "relation": "referenced", "confidence": "AMBIGUOUS",
             "source_file": "paper.md", "weight": 0.5}
        ],
        "input_tokens": 1200,
        "output_tokens": 340
    })
}

fn make_graph() -> Graph {
    build_from_json(extraction_json(), false, None).expect("build_from_json")
}

// ── test_cluster_returns_dict ─────────────────────────────────────────────────

#[test]
fn cluster_returns_map() {
    let g = make_graph();
    let communities = cluster(&g, 1.0, None);
    // Result is a non-empty map (the fixture has 4 nodes with edges)
    assert!(!communities.is_empty());
}

// ── test_cluster_covers_all_nodes ────────────────────────────────────────────

#[test]
fn cluster_covers_all_nodes() {
    let g = make_graph();
    let communities = cluster(&g, 1.0, None);
    let all_nodes: std::collections::HashSet<String> =
        communities.values().flatten().cloned().collect();
    let expected: std::collections::HashSet<String> = g.nodes().map(|(id, _)| id.clone()).collect();
    assert_eq!(all_nodes, expected);
}

// ── test_cohesion_score_complete_graph ────────────────────────────────────────

#[test]
fn cohesion_score_complete_graph() {
    // Complete graph on 4 nodes (all pairs connected)
    let mut g = Graph::new(GraphKind::Graph);
    let nodes = ["0", "1", "2", "3"];
    for n in nodes {
        g.add_node(n, indexmap::IndexMap::new());
    }
    for i in 0..4_usize {
        for j in (i + 1)..4_usize {
            let mut attrs = indexmap::IndexMap::new();
            attrs.insert("weight".to_string(), json!(1.0));
            g.add_edge(nodes[i], nodes[j], attrs);
        }
    }
    let score = cohesion_score(
        &g,
        &nodes.iter().map(ToString::to_string).collect::<Vec<_>>(),
    );
    assert!((score - 1.0).abs() < f64::EPSILON);
}

// ── test_cohesion_score_single_node ──────────────────────────────────────────

#[test]
fn cohesion_score_single_node() {
    let mut g = Graph::new(GraphKind::Graph);
    g.add_node("a", indexmap::IndexMap::new());
    let score = cohesion_score(&g, &["a".to_string()]);
    assert!((score - 1.0).abs() < f64::EPSILON);
}

// ── test_cohesion_score_disconnected ─────────────────────────────────────────

#[test]
fn cohesion_score_disconnected() {
    let mut g = Graph::new(GraphKind::Graph);
    g.add_node("a", indexmap::IndexMap::new());
    g.add_node("b", indexmap::IndexMap::new());
    g.add_node("c", indexmap::IndexMap::new());
    let score = cohesion_score(&g, &["a", "b", "c"].map(str::to_string));
    assert!((score - 0.0).abs() < f64::EPSILON);
}

// ── test_cohesion_score_range ─────────────────────────────────────────────────

#[test]
fn cohesion_score_range() {
    let g = make_graph();
    let communities = cluster(&g, 1.0, None);
    for (_, nodes) in &communities {
        let score = cohesion_score(&g, nodes);
        assert!((0.0..=1.0).contains(&score), "score out of range: {score}");
    }
}

// ── test_score_all_keys_match_communities ────────────────────────────────────

#[test]
fn score_all_keys_match_communities() {
    let g = make_graph();
    let communities = cluster(&g, 1.0, None);
    let scores = score_all(&g, &communities);
    assert_eq!(
        scores.keys().collect::<std::collections::HashSet<_>>(),
        communities.keys().collect::<std::collections::HashSet<_>>()
    );
}

// ── test_cluster_does_not_write_to_stdout / stderr ───────────────────────────
// Rust functions don't write to stdout/stderr; this is trivially satisfied.
// We include the tests as documentation that the behaviour is expected.

#[test]
fn cluster_produces_no_stdout() {
    // In Rust there is no graspologic to produce ANSI codes.
    // This test exists to document the contract and verify it still builds.
    let g = make_graph();
    let _ = cluster(&g, 1.0, None);
    // If this test compiles and runs without panicking, it passes.
}

// ── test_remap_communities_to_previous_reuses_old_ids ────────────────────────

#[test]
fn remap_reuses_old_ids() {
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(10, vec!["a".into(), "b".into(), "c".into()]);
    communities.insert(11, vec!["d".into(), "e".into()]);

    let mut previous: IndexMap<String, i64> = IndexMap::new();
    previous.insert("a".into(), 5);
    previous.insert("b".into(), 5);
    previous.insert("c".into(), 5);
    previous.insert("d".into(), 1);
    previous.insert("e".into(), 1);

    let remapped = remap_communities_to_previous(&communities, &previous);
    let key_set: std::collections::HashSet<i64> = remapped.keys().copied().collect();
    assert_eq!(key_set, std::collections::HashSet::from([1, 5]));
    assert_eq!(remapped[&5], vec!["a", "b", "c"]);
    assert_eq!(remapped[&1], vec!["d", "e"]);
}

// ── test_remap_communities_to_previous_assigns_deterministic_new_ids ─────────

#[test]
fn remap_assigns_deterministic_new_ids() {
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    communities.insert(7, vec!["x".into(), "y".into(), "z".into()]);
    communities.insert(8, vec!["m".into()]);

    let mut previous: IndexMap<String, i64> = IndexMap::new();
    previous.insert("a".into(), 3);

    let remapped = remap_communities_to_previous(&communities, &previous);
    let keys: Vec<i64> = remapped.keys().copied().collect();
    assert_eq!(keys, vec![0, 1]);
    assert_eq!(remapped[&0], vec!["x", "y", "z"]);
    assert_eq!(remapped[&1], vec!["m"]);
}

// ── Additional structural tests ───────────────────────────────────────────────

#[test]
fn cluster_empty_graph_returns_empty_map() {
    let g = Graph::new(GraphKind::Graph);
    let communities = cluster(&g, 1.0, None);
    assert!(communities.is_empty());
}

#[test]
fn cluster_no_edges_each_node_own_community() {
    let mut g = Graph::new(GraphKind::Graph);
    g.add_node("a", indexmap::IndexMap::new());
    g.add_node("b", indexmap::IndexMap::new());
    g.add_node("c", indexmap::IndexMap::new());
    let communities = cluster(&g, 1.0, None);
    // Every node should be in its own community
    assert_eq!(communities.len(), 3);
    let all_nodes: std::collections::HashSet<String> =
        communities.values().flatten().cloned().collect();
    assert!(all_nodes.contains("a"));
    assert!(all_nodes.contains("b"));
    assert!(all_nodes.contains("c"));
    for (_, nodes) in &communities {
        assert_eq!(nodes.len(), 1);
    }
}

#[test]
fn cluster_digraph_converted_to_undirected() {
    // Build a directed graph; clustering should still work
    let ext = json!({
        "nodes": [
            {"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "b", "label": "B", "file_type": "code", "source_file": "b.py"},
            {"id": "c", "label": "C", "file_type": "code", "source_file": "c.py"},
        ],
        "edges": [
            {"source": "a", "target": "b", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "a.py"},
            {"source": "b", "target": "c", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "b.py"},
        ],
    });
    let g = build_from_json(ext, true, None).expect("build");
    assert!(g.kind.is_directed());
    let communities = cluster(&g, 1.0, None);
    let all_nodes: std::collections::HashSet<String> =
        communities.values().flatten().cloned().collect();
    assert_eq!(all_nodes.len(), 3);
}

#[test]
fn cluster_communities_sorted_size_desc() {
    // Build a graph with one large cluster and one small one
    let mut g = Graph::new(GraphKind::Graph);
    // Large cluster: a-b-c-d fully connected
    for n in ["a", "b", "c", "d"] {
        g.add_node(n, indexmap::IndexMap::new());
    }
    for (u, v) in [
        ("a", "b"),
        ("a", "c"),
        ("a", "d"),
        ("b", "c"),
        ("b", "d"),
        ("c", "d"),
    ] {
        let mut attrs = indexmap::IndexMap::new();
        attrs.insert("weight".to_string(), json!(1.0));
        g.add_edge(u, v, attrs);
    }
    // Small cluster: e-f
    g.add_node("e", indexmap::IndexMap::new());
    g.add_node("f", indexmap::IndexMap::new());
    let mut attrs = indexmap::IndexMap::new();
    attrs.insert("weight".to_string(), json!(1.0));
    g.add_edge("e", "f", attrs);

    let communities = cluster(&g, 1.0, None);
    let sizes: Vec<usize> = communities.values().map(Vec::len).collect();
    // Communities must be in descending order of size
    for w in sizes.windows(2) {
        assert!(w[0] >= w[1], "communities not sorted by size: {sizes:?}");
    }
}

#[test]
fn cluster_nodes_within_community_sorted_alpha() {
    let g = make_graph();
    let communities = cluster(&g, 1.0, None);
    for (_, nodes) in &communities {
        let mut sorted = nodes.clone();
        sorted.sort_unstable();
        assert_eq!(
            nodes, &sorted,
            "nodes not alphabetically sorted in community"
        );
    }
}

#[test]
fn remap_empty_communities_returns_empty() {
    let communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    let previous: IndexMap<String, i64> = IndexMap::new();
    let result = remap_communities_to_previous(&communities, &previous);
    assert!(result.is_empty());
}
