//! God-node detection (highest-degree real entities).
//!
//! Extracted from `lib.rs` to isolate the `god_nodes` public function and
//! its dependency on degree computation and node classification.

use graphify_build::Graph;
use serde_json::{Value, json};
use std::cmp::Reverse;

use crate::centrality::all_degrees;
use crate::classify::{is_concept_node, is_file_node, is_json_key_node};

/// Return the top-`top_n` most-connected real entities (god nodes).
///
/// File-level hub nodes, concept nodes, and JSON key noise nodes are excluded.
///
/// Mirrors Python `god_nodes`.
#[must_use]
pub fn god_nodes(graph: &Graph, top_n: usize) -> Vec<Value> {
    let degrees = all_degrees(graph);
    let mut sorted: Vec<(&String, usize)> = degrees.iter().map(|(id, &d)| (id, d)).collect();
    sorted.sort_by_key(|item| Reverse(item.1));

    let mut result = Vec::new();
    for (node_id, deg) in sorted {
        if is_file_node(graph, node_id)
            || is_concept_node(graph, node_id)
            || is_json_key_node(graph, node_id)
        {
            continue;
        }
        let label = graph
            .node_data(node_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(node_id);
        result.push(json!({
            "id": node_id,
            "label": label,
            "degree": deg,
        }));
        if result.len() >= top_n {
            break;
        }
    }
    result
}
