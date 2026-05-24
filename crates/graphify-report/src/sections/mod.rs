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

use std::collections::HashMap;

use graphify_build::Graph;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Node classification helpers (mirrors Python `analyze._is_file_node` /
// `_is_concept_node`).  These live here because `graphify-analyze` is a stub
// and `report.py` calls them directly on `G`.
// ---------------------------------------------------------------------------

/// Precomputed per-node degree map. Building it is `O(E)`; every consumer
/// (`is_file_node`, isolated-node detection, etc.) then runs in `O(1)` per
/// lookup. The previous shape called `node_degree(graph, id)` inside hot
/// loops — `O(N × E)` total — which on a 25k-node / 36k-edge corpus added
/// ~2s of pure waste to every report render.
#[must_use]
pub(crate) fn compute_degrees(graph: &Graph) -> HashMap<String, usize> {
    let mut degrees: HashMap<String, usize> = HashMap::with_capacity(graph.node_count());
    for edge in graph.edges() {
        *degrees.entry(edge.source.clone()).or_insert(0) += 1;
        if edge.target != edge.source {
            *degrees.entry(edge.target.clone()).or_insert(0) += 1;
        }
    }
    degrees
}

/// Return `true` if `node_id` is a structural file hub, method stub, or
/// low-degree function stub — rather than a real semantic entity.
///
/// Requires a precomputed `degrees` map (see [`compute_degrees`]). The
/// previous version called `node_degree(graph, node_id)` inline, which on
/// large graphs was the dominant cost of report generation.
///
/// These nodes are excluded from god-node lists, surprise scores, and community
/// displays.  Mirrors `graphify-py/graphify/analyze.py` `_is_file_node`.
pub(crate) fn is_file_node(graph: &Graph, node_id: &str, degrees: &HashMap<String, usize>) -> bool {
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
    if label.ends_with("()") && degrees.get(node_id).copied().unwrap_or(0) <= 1 {
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
    // Python raises `KeyError` for an unknown id; the safer Rust analogue
    // is to report `false` so callers don't silently lump truly-missing
    // nodes into the "concept" bucket.
    let Some(attrs) = graph.node_data(node_id) else {
        return false;
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
