//! Coverage tests for the MCP tool handler functions.

#![allow(clippy::expect_used)]

use std::collections::HashMap;

use graphify_build::{Graph, build_from_json};
use graphify_serve::graph::communities_from_graph;
use graphify_serve::tools::{
    tool_get_community, tool_get_neighbors, tool_get_node, tool_god_nodes, tool_graph_stats,
    tool_query_graph, tool_shortest_path,
};
use serde_json::{Map, Value, json};

fn graph_with_data() -> Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "alpha", "source_file": "alpha.py",
                 "source_location": "L1", "community": 0, "file_type": "code"},
                {"id": "n2", "label": "beta", "source_file": "beta.py",
                 "source_location": "L1", "community": 0, "file_type": "code"},
                {"id": "n3", "label": "gamma", "source_file": "gamma.py",
                 "source_location": "L1", "community": 1, "file_type": "code"},
                {"id": "n4", "label": "delta", "source_file": "delta.py",
                 "source_location": "L1", "community": 1, "file_type": "code"},
            ],
            "edges": [
                {"source": "n1", "target": "n2", "relation": "calls",
                 "confidence": "EXTRACTED"},
                {"source": "n2", "target": "n3", "relation": "imports",
                 "confidence": "EXTRACTED"},
                {"source": "n3", "target": "n4", "relation": "uses",
                 "confidence": "INFERRED"},
                {"source": "n4", "target": "n1", "relation": "depends",
                 "confidence": "AMBIGUOUS"},
            ]
        }),
        false,
        None,
    )
    .expect("test invariant")
}

fn arg_map(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

// ── tool_get_node ───────────────────────────────────────────────────────────

#[test]
fn tool_get_node_label_match() {
    let g = graph_with_data();
    let args = arg_map(&[("label", json!("alpha"))]);
    let out = tool_get_node(&g, &args);
    assert!(out.contains("alpha"), "got: {out}");
    assert!(out.contains("Degree"));
}

#[test]
fn tool_get_node_no_label_argument() {
    let g = graph_with_data();
    let args = arg_map(&[]);
    let out = tool_get_node(&g, &args);
    assert!(out.starts_with("Error"));
}

#[test]
fn tool_get_node_no_match() {
    let g = graph_with_data();
    let args = arg_map(&[("label", json!("zzznotfound"))]);
    let out = tool_get_node(&g, &args);
    assert!(out.contains("No node matching"));
}

// ── tool_get_neighbors ──────────────────────────────────────────────────────

#[test]
fn tool_get_neighbors_label_match() {
    let g = graph_with_data();
    let args = arg_map(&[("label", json!("alpha"))]);
    let out = tool_get_neighbors(&g, &args);
    assert!(out.contains("Neighbors"));
}

#[test]
fn tool_get_neighbors_with_relation_filter() {
    let g = graph_with_data();
    let args = arg_map(&[
        ("label", json!("alpha")),
        ("relation_filter", json!("calls")),
    ]);
    let out = tool_get_neighbors(&g, &args);
    assert!(out.contains("Neighbors"));
}

#[test]
fn tool_get_neighbors_no_label() {
    let g = graph_with_data();
    let args = arg_map(&[]);
    let out = tool_get_neighbors(&g, &args);
    assert!(out.starts_with("Error"));
}

#[test]
fn tool_get_neighbors_no_match() {
    let g = graph_with_data();
    let args = arg_map(&[("label", json!("nothing"))]);
    let out = tool_get_neighbors(&g, &args);
    assert!(out.contains("No node matching"));
}

// ── tool_get_community ──────────────────────────────────────────────────────

#[test]
fn tool_get_community_found() {
    let g = graph_with_data();
    let communities = communities_from_graph(&g);
    let args = arg_map(&[("community_id", json!(0))]);
    let out = tool_get_community(&g, &communities, &args);
    assert!(out.contains("Community 0"));
}

#[test]
fn tool_get_community_missing_arg() {
    let g = graph_with_data();
    let communities = communities_from_graph(&g);
    let args = arg_map(&[]);
    let out = tool_get_community(&g, &communities, &args);
    assert!(out.starts_with("Error"));
}

#[test]
fn tool_get_community_not_found() {
    let g = graph_with_data();
    let communities = communities_from_graph(&g);
    let args = arg_map(&[("community_id", json!(99))]);
    let out = tool_get_community(&g, &communities, &args);
    assert!(out.contains("not found"));
}

// ── tool_god_nodes ──────────────────────────────────────────────────────────

#[test]
fn tool_god_nodes_default_top_n() {
    let g = graph_with_data();
    let args = arg_map(&[]);
    let out = tool_god_nodes(&g, &args);
    assert!(out.starts_with("God nodes"));
}

#[test]
fn tool_god_nodes_explicit_top_n() {
    let g = graph_with_data();
    let args = arg_map(&[("top_n", json!(2))]);
    let out = tool_god_nodes(&g, &args);
    assert!(out.starts_with("God nodes"));
}

// ── tool_graph_stats ────────────────────────────────────────────────────────

#[test]
fn tool_graph_stats_renders_counts() {
    let g = graph_with_data();
    let communities = communities_from_graph(&g);
    let out = tool_graph_stats(&g, &communities);
    assert!(out.contains("Nodes:"));
    assert!(out.contains("Edges:"));
    assert!(out.contains("EXTRACTED:"));
    assert!(out.contains("INFERRED:"));
    assert!(out.contains("AMBIGUOUS:"));
}

// ── tool_shortest_path ──────────────────────────────────────────────────────

#[test]
fn tool_shortest_path_finds_path() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[("source", json!("alpha")), ("target", json!("gamma"))]);
    let out = tool_shortest_path(&g, &args, &mut cache);
    assert!(out.contains("alpha"));
}

#[test]
fn tool_shortest_path_missing_source() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[("target", json!("beta"))]);
    let out = tool_shortest_path(&g, &args, &mut cache);
    assert!(out.starts_with("Error"));
}

#[test]
fn tool_shortest_path_missing_target() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[("source", json!("alpha"))]);
    let out = tool_shortest_path(&g, &args, &mut cache);
    assert!(out.starts_with("Error"));
}

#[test]
fn tool_shortest_path_source_not_found() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[("source", json!("zzznone")), ("target", json!("alpha"))]);
    let out = tool_shortest_path(&g, &args, &mut cache);
    assert!(out.contains("No node matching"));
}

#[test]
fn tool_shortest_path_target_not_found() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[("source", json!("alpha")), ("target", json!("zzznone"))]);
    let out = tool_shortest_path(&g, &args, &mut cache);
    assert!(out.contains("No node matching"));
}

#[test]
fn tool_shortest_path_same_node() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[("source", json!("alpha")), ("target", json!("alpha"))]);
    let out = tool_shortest_path(&g, &args, &mut cache);
    assert!(out.contains("both resolved"));
}

// ── tool_query_graph ────────────────────────────────────────────────────────

#[test]
fn tool_query_graph_runs() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[("question", json!("how does alpha use beta?"))]);
    let out = tool_query_graph(&g, &args, &mut cache);
    assert!(!out.is_empty());
}

#[test]
fn tool_query_graph_missing_question() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[]);
    let out = tool_query_graph(&g, &args, &mut cache);
    assert!(out.starts_with("Error"));
}

#[test]
fn tool_query_graph_with_filters() {
    let g = graph_with_data();
    let mut cache = HashMap::new();
    let args = arg_map(&[
        ("question", json!("trace alpha")),
        ("mode", json!("dfs")),
        ("depth", json!(2)),
        ("token_budget", json!(1500)),
        ("context_filter", json!(["call"])),
    ]);
    let out = tool_query_graph(&g, &args, &mut cache);
    assert!(!out.is_empty());
}
