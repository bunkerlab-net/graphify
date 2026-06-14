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

/// Builtin / mock / stdlib labels that can appear as annotation-derived nodes
/// in pre-existing graphs. Excluded from god-node ranking so they don't
/// displace real abstractions even when they were not filtered at extraction
/// time (#1147). Matched case-sensitively against the raw node label.
static BUILTIN_NOISE_LABELS: LazyLock<IndexSet<&'static str>> = LazyLock::new(|| {
    [
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
        "MagicMock",
        "Mock",
        "AsyncMock",
        "NonCallableMock",
        "NonCallableMagicMock",
        "PropertyMock",
        "patch",
        "sentinel",
        // Stdlib types commonly confused for project symbols.
        "Path",
        "Any",
        "Optional",
        "List",
        "Dict",
        "Set",
        "Tuple",
        "Union",
        "Callable",
        "Type",
        "ClassVar",
        "Final",
        "Literal",
        "Protocol",
        "Counter",
        "defaultdict",
        "OrderedDict",
        "datetime",
        "Enum",
        "os",
        "sys",
        "re",
        "json",
        "io",
        "abc",
        "typing",
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
