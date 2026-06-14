//! God-node detection (highest-degree real entities).
//!
//! Extracted from `lib.rs` to isolate the `god_nodes` public function and
//! its dependency on degree computation and node classification.

use graphify_build::Graph;
use indexmap::IndexSet;
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::sync::LazyLock;

use crate::centrality::all_degrees;
use crate::classify::{is_concept_node, is_file_node, is_json_key_node};

/// Scalar builtins and `unittest.mock` labels that can appear as
/// annotation-derived nodes in pre-existing graphs. Excluded from god-node
/// ranking so they don't displace real abstractions even when they were not
/// filtered at extraction time (#1147). Matched case-sensitively against the
/// raw node label.
///
/// This mirrors graphify-py `_BUILTIN_NOISE_LABELS` exactly. It deliberately
/// does NOT include stdlib container/module names (`Path`, `os`, `datetime`,
/// `Enum`, …): those can be legitimate high-degree project concerns, the
/// reference does not filter them, and filtering them here would diverge from
/// the extract-time `PYTHON_ANNOTATION_NOISE` set (which is also exactly this
/// list). Keeping the two sets identical means a node either survives both
/// filters or neither.
static BUILTIN_NOISE_LABELS: LazyLock<IndexSet<&'static str>> = LazyLock::new(|| {
    [
        // scalar builtins
        "str",
        "int",
        "float",
        "bool",
        "bytes",
        "bytearray",
        "complex",
        "object",
        "True",
        "False",
        // unittest.mock
        "MagicMock",
        "Mock",
        "AsyncMock",
        "NonCallableMock",
        "NonCallableMagicMock",
        "PropertyMock",
        "patch",
        "sentinel",
    ]
    .into_iter()
    .collect()
});

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
        if is_file_node(graph, node_id, &degrees)
            || is_concept_node(graph, node_id)
            || is_json_key_node(graph, node_id)
        {
            continue;
        }
        let label_opt = graph
            .node_data(node_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str);
        // Drop builtin / mock / stdlib annotation noise (#1147).
        if BUILTIN_NOISE_LABELS.contains(label_opt.unwrap_or("")) {
            continue;
        }
        let label = label_opt.unwrap_or(node_id);
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
