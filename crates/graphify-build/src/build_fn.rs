//! Top-level [`build_from_json`] and [`build`] drivers.

use std::path::Path;
use std::sync::LazyLock;

use serde_json::Value;

use crate::dedup_label::deduplicate_by_label;
use crate::error::BuildError;
use crate::graph::{Graph, GraphKind};
use crate::ingest::{add_edges, add_nodes, canonicalise_nodes};

static PERF_LOG: LazyLock<bool> = LazyLock::new(|| std::env::var("GRAPHIFY_PERF_LOG").is_ok());

/// Build a graph from a single extraction dict.
///
/// Mirrors Python `build_from_json(extraction, directed=False, root=None)`.
///
/// The function:
/// 1. Renames `"links"` → `"edges"` for compatibility with `NetworkX` dumps.
/// 2. Canonicalises node `file_type` values and renames `source` →
///    `source_file`.
/// 3. Runs the validator and surfaces real schema warnings on stderr
///    (dangling-edge warnings are suppressed since stdlib/external
///    imports are expected).
/// 4. Inserts nodes and edges into a fresh [`Graph`].
/// 5. Preserves `hyperedges` on `graph.graph_attrs` for downstream
///    consumers.
///
/// # Errors
///
/// Currently infallible (returns `Result` for API parity with [`build`],
/// which can fail with [`BuildError::WouldShrink`]).
pub fn build_from_json(
    mut extraction: Value,
    directed: bool,
    root: Option<&Path>,
) -> Result<Graph, BuildError> {
    let root_str = root.map(|p| {
        p.canonicalize()
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .into_owned()
    });
    let kind = if directed {
        GraphKind::DiGraph
    } else {
        GraphKind::Graph
    };

    let Some(obj) = extraction.as_object_mut() else {
        return Ok(Graph::new(kind));
    };
    if !obj.contains_key("edges")
        && let Some(links) = obj.remove("links")
    {
        obj.insert("edges".into(), links);
    }

    let perf = *PERF_LOG;
    let t = std::time::Instant::now();
    canonicalise_nodes(&mut extraction);
    if perf {
        eprintln!(
            "[perf]   build_from_json/canonicalise: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    // Mirror Python `build.py:148-152`: surface real schema errors, but ignore
    // dangling-edge warnings (stdlib/external imports are expected).
    let errors = graphify_validate::validate_extraction(&extraction);
    if perf {
        eprintln!(
            "[perf]   build_from_json/validate: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let real_errors: Vec<&String> = errors
        .iter()
        .filter(|e| !e.contains("does not match any node id"))
        .collect();
    if let Some(first) = real_errors.first() {
        eprintln!(
            "[graphify] Extraction warning ({} issues): {first}",
            real_errors.len()
        );
    }

    let mut graph = Graph::new(kind);
    let t = std::time::Instant::now();
    add_nodes(&mut graph, &mut extraction, root_str.as_deref());
    if perf {
        eprintln!(
            "[perf]   build_from_json/add_nodes: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    add_edges(&mut graph, &extraction, root_str.as_deref());
    if perf {
        eprintln!(
            "[perf]   build_from_json/add_edges: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }

    if let Some(hyperedges) = extraction
        .as_object()
        .and_then(|o| o.get("hyperedges"))
        .cloned()
        && let Some(arr) = hyperedges.as_array()
        && !arr.is_empty()
    {
        graph
            .graph_attrs
            .insert("hyperedges".to_string(), hyperedges);
    }

    Ok(graph)
}

/// Merge multiple extraction dicts into one graph. Mirrors Python
/// `build(...)`.
///
/// `dedup` runs entity deduplication via [`deduplicate_by_label`]. The
/// Python version optionally also calls
/// `graphify.dedup.deduplicate_entities` for LLM-assisted fuzzy
/// matching — that path requires the `graphify-dedup` crate and is
/// opt-in via `dedup_llm_backend`. For now we only support the cheap
/// label-canonical dedup path; LLM-backed dedup is reserved for future
/// work.
///
/// # Errors
///
/// Propagates any error from [`build_from_json`].
pub fn build(
    extractions: &[Value],
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
) -> Result<Graph, BuildError> {
    let mut combined_nodes: Vec<Value> = Vec::new();
    let mut combined_edges: Vec<Value> = Vec::new();
    let mut combined_hyperedges: Vec<Value> = Vec::new();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;

    for ext in extractions {
        if let Some(arr) = ext.get("nodes").and_then(Value::as_array) {
            combined_nodes.extend(arr.iter().cloned());
        }
        if let Some(arr) = ext.get("edges").and_then(Value::as_array) {
            combined_edges.extend(arr.iter().cloned());
        } else if let Some(arr) = ext.get("links").and_then(Value::as_array) {
            combined_edges.extend(arr.iter().cloned());
        }
        if let Some(arr) = ext.get("hyperedges").and_then(Value::as_array) {
            combined_hyperedges.extend(arr.iter().cloned());
        }
        if let Some(n) = ext.get("input_tokens").and_then(Value::as_u64) {
            input_tokens += n;
        }
        if let Some(n) = ext.get("output_tokens").and_then(Value::as_u64) {
            output_tokens += n;
        }
    }

    if dedup && !combined_nodes.is_empty() {
        let (nodes, edges) = deduplicate_by_label(&combined_nodes, &combined_edges);
        combined_nodes = nodes;
        combined_edges = edges;
    }

    let combined = serde_json::json!({
        "nodes": combined_nodes,
        "edges": combined_edges,
        "hyperedges": combined_hyperedges,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
    });
    build_from_json(combined, directed, root)
}
