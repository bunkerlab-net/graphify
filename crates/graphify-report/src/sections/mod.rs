//! Section renderers for `GRAPH_REPORT.md`.
//!
//! Each sub-module owns one logical section of the report.  Shared node
//! classification helpers live here because multiple sections need them.

pub mod communities;
pub mod detection;
pub mod god_nodes;
pub mod header;
pub mod suggestions;
pub mod surprises;
pub mod tokens;

use graphify_build::Graph;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Node classification helpers (mirrors Python `analyze._is_file_node` /
// `_is_concept_node`).  These live here because `graphify-analyze` is a stub
// and `report.py` calls them directly on `G`.
// ---------------------------------------------------------------------------

/// Return `true` if `node_id` is a structural file hub, method stub, or
/// low-degree function stub — rather than a real semantic entity.
///
/// These nodes are excluded from god-node lists, surprise scores, and community
/// displays.  Mirrors `graphify-py/graphify/analyze.py` `_is_file_node`.
pub(crate) fn is_file_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return false;
    };
    let label = attrs
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if label.is_empty() {
        return false;
    }
    // File-level hub: label matches the source filename.
    let source_file = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !source_file.is_empty() {
        let filename = source_file.rsplit('/').next().unwrap_or(source_file);
        if label == filename {
            return true;
        }
    }
    // Method stub: AST extractor labels methods as `.method_name()`.
    if label.starts_with('.') && label.ends_with("()") {
        return true;
    }
    // Module-level function stub: `name()` with degree <= 1.
    if label.ends_with("()") && node_degree(graph, node_id) <= 1 {
        return true;
    }
    false
}

/// Return `true` if `node_id` is a manually-injected concept with no real
/// source file (empty `source_file` or a path with no extension).
///
/// Concept nodes should not be filtered out from reports (they are real
/// semantic entities), but they are excluded from cross-file surprise
/// detection.  Mirrors `graphify-py/graphify/analyze.py` `_is_concept_node`.
pub(crate) fn is_concept_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return true;
    };
    let source = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if source.is_empty() {
        return true;
    }
    // No file extension in the last path component → concept label, not a real file.
    let last = source.rsplit('/').next().unwrap_or(source);
    !last.contains('.')
}

/// Count how many edges involve `node_id` (undirected degree).
pub(crate) fn node_degree(graph: &Graph, node_id: &str) -> usize {
    graph
        .edges()
        .filter(|e| e.source == node_id || e.target == node_id)
        .count()
}
