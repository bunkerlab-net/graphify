//! Graph diff — compares two graph snapshots and reports what changed.
//!
//! Extracted from `lib.rs` to isolate `graph_diff`, which returns a JSON
//! object with `new_nodes`, `removed_nodes`, `new_edges`, `removed_edges`,
//! and a human-readable `summary` string.

use graphify_build::Graph;
use indexmap::IndexSet;
use serde_json::{Value, json};

/// Compare two graph snapshots and return what changed.
///
/// Returns a JSON object with `new_nodes`, `removed_nodes`, `new_edges`,
/// `removed_edges`, and a `summary` string.
///
/// Mirrors Python `graph_diff`.
#[must_use]
pub fn graph_diff(graph_old: &Graph, graph_new: &Graph) -> Value {
    let (new_nodes, removed_nodes) = diff_nodes(graph_old, graph_new);
    let directed = graph_old.kind.is_directed() || graph_new.kind.is_directed();
    let (new_edges, removed_edges) = diff_edges(graph_old, graph_new, directed);
    let summary = build_summary(
        new_nodes.len(),
        new_edges.len(),
        removed_nodes.len(),
        removed_edges.len(),
    );
    json!({
        "new_nodes": new_nodes,
        "removed_nodes": removed_nodes,
        "new_edges": new_edges,
        "removed_edges": removed_edges,
        "summary": summary,
    })
}

/// Compute new vs removed nodes as `(id, label)` JSON pairs.
fn diff_nodes(graph_old: &Graph, graph_new: &Graph) -> (Vec<Value>, Vec<Value>) {
    let old_ids: IndexSet<&str> = graph_old.nodes().map(|(id, _)| id.as_str()).collect();
    let new_ids: IndexSet<&str> = graph_new.nodes().map(|(id, _)| id.as_str()).collect();

    let added: Vec<Value> = new_ids
        .iter()
        .filter(|id| !old_ids.contains(*id))
        .map(|&id| node_entry(graph_new, id))
        .collect();
    let removed: Vec<Value> = old_ids
        .iter()
        .filter(|id| !new_ids.contains(*id))
        .map(|&id| node_entry(graph_old, id))
        .collect();
    (added, removed)
}

/// Build a `{"id": ..., "label": ...}` entry, falling back to the id when no label.
fn node_entry(graph: &Graph, id: &str) -> Value {
    let label = graph
        .node_data(id)
        .and_then(|a| a.get("label"))
        .and_then(Value::as_str)
        .unwrap_or(id);
    json!({"id": id, "label": label})
}

/// Compute new vs removed edges using a canonical edge key (undirected = sorted endpoints).
fn diff_edges(graph_old: &Graph, graph_new: &Graph, directed: bool) -> (Vec<Value>, Vec<Value>) {
    let old_keys = edge_key_set(graph_old, directed);
    let new_keys = edge_key_set(graph_new, directed);
    let new_edges = edges_missing_from(graph_new, &old_keys, directed);
    let removed_edges = edges_missing_from(graph_old, &new_keys, directed);
    (new_edges, removed_edges)
}

/// Canonical edge key: directed = (u,v,relation); undirected = sorted endpoints.
fn make_edge_key(u: &str, v: &str, relation: &str, directed: bool) -> (String, String, String) {
    if directed {
        (u.to_string(), v.to_string(), relation.to_string())
    } else {
        let (a, b) = if u <= v { (u, v) } else { (v, u) };
        (a.to_string(), b.to_string(), relation.to_string())
    }
}

/// Collect the canonical edge-key set from a graph.
fn edge_key_set(graph: &Graph, directed: bool) -> IndexSet<(String, String, String)> {
    graph
        .edges()
        .map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            make_edge_key(&e.source, &e.target, rel, directed)
        })
        .collect()
}

/// Collect edges in `graph` whose canonical key is missing from `other_keys`.
fn edges_missing_from(
    graph: &Graph,
    other_keys: &IndexSet<(String, String, String)>,
    directed: bool,
) -> Vec<Value> {
    graph
        .edges()
        .filter_map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = make_edge_key(&e.source, &e.target, rel, directed);
            if other_keys.contains(&key) {
                return None;
            }
            let conf = e
                .attrs
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("");
            Some(json!({
                "source": e.source,
                "target": e.target,
                "relation": rel,
                "confidence": conf,
            }))
        })
        .collect()
}

/// Build the human-readable "1 new node, 2 edges removed" summary string.
fn build_summary(
    new_nodes: usize,
    new_edges: usize,
    removed_nodes: usize,
    removed_edges: usize,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if new_nodes > 0 {
        parts.push(format!(
            "{new_nodes} new node{}",
            if new_nodes == 1 { "" } else { "s" }
        ));
    }
    if new_edges > 0 {
        parts.push(format!(
            "{new_edges} new edge{}",
            if new_edges == 1 { "" } else { "s" }
        ));
    }
    if removed_nodes > 0 {
        parts.push(format!(
            "{removed_nodes} node{} removed",
            if removed_nodes == 1 { "" } else { "s" }
        ));
    }
    if removed_edges > 0 {
        parts.push(format!(
            "{removed_edges} edge{} removed",
            if removed_edges == 1 { "" } else { "s" }
        ));
    }
    if parts.is_empty() {
        "no changes".to_string()
    } else {
        parts.join(", ")
    }
}
