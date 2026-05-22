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
#[allow(clippy::too_many_lines)] // diff collects four lists independently; splitting adds no clarity
pub fn graph_diff(graph_old: &Graph, graph_new: &Graph) -> Value {
    let old_node_ids: IndexSet<&str> = graph_old.nodes().map(|(id, _)| id.as_str()).collect();
    let new_node_ids: IndexSet<&str> = graph_new.nodes().map(|(id, _)| id.as_str()).collect();

    let added_ids: Vec<&str> = new_node_ids
        .iter()
        .filter(|id| !old_node_ids.contains(*id))
        .copied()
        .collect();
    let removed_ids: Vec<&str> = old_node_ids
        .iter()
        .filter(|id| !new_node_ids.contains(*id))
        .copied()
        .collect();

    let new_nodes: Vec<Value> = added_ids
        .iter()
        .map(|&id| {
            let label = graph_new
                .node_data(id)
                .and_then(|a| a.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(id);
            json!({"id": id, "label": label})
        })
        .collect();

    let removed_nodes: Vec<Value> = removed_ids
        .iter()
        .map(|&id| {
            let label = graph_old
                .node_data(id)
                .and_then(|a| a.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(id);
            json!({"id": id, "label": label})
        })
        .collect();

    // Edge key function: (min(u,v), max(u,v), relation) for undirected
    let directed = graph_old.kind.is_directed() || graph_new.kind.is_directed();
    let edge_key = |u: &str, v: &str, relation: &str| -> (String, String, String) {
        if directed {
            (u.to_string(), v.to_string(), relation.to_string())
        } else {
            let (a, b) = if u <= v { (u, v) } else { (v, u) };
            (a.to_string(), b.to_string(), relation.to_string())
        }
    };

    let old_edge_keys: IndexSet<(String, String, String)> = graph_old
        .edges()
        .map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            edge_key(&e.source, &e.target, rel)
        })
        .collect();

    let new_edge_keys: IndexSet<(String, String, String)> = graph_new
        .edges()
        .map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            edge_key(&e.source, &e.target, rel)
        })
        .collect();

    let new_edges: Vec<Value> = graph_new
        .edges()
        .filter_map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = edge_key(&e.source, &e.target, rel);
            if old_edge_keys.contains(&key) {
                None
            } else {
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
            }
        })
        .collect();

    let removed_edges: Vec<Value> = graph_old
        .edges()
        .filter_map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = edge_key(&e.source, &e.target, rel);
            if new_edge_keys.contains(&key) {
                None
            } else {
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
            }
        })
        .collect();

    // Build summary
    let mut parts: Vec<String> = Vec::new();
    let nn = new_nodes.len();
    let ne = new_edges.len();
    let rn = removed_nodes.len();
    let re = removed_edges.len();
    if nn > 0 {
        parts.push(format!("{nn} new node{}", if nn == 1 { "" } else { "s" }));
    }
    if ne > 0 {
        parts.push(format!("{ne} new edge{}", if ne == 1 { "" } else { "s" }));
    }
    if rn > 0 {
        parts.push(format!(
            "{rn} node{} removed",
            if rn == 1 { "" } else { "s" }
        ));
    }
    if re > 0 {
        parts.push(format!(
            "{re} edge{} removed",
            if re == 1 { "" } else { "s" }
        ));
    }
    let summary = if parts.is_empty() {
        "no changes".to_string()
    } else {
        parts.join(", ")
    };

    json!({
        "new_nodes": new_nodes,
        "removed_nodes": removed_nodes,
        "new_edges": new_edges,
        "removed_edges": removed_edges,
        "summary": summary,
    })
}
