//! Top-level [`build_from_json`] and [`build`] drivers.

use std::path::Path;
use std::sync::LazyLock;

use serde_json::Value;

use crate::dedup_label::deduplicate_by_label;
use crate::error::BuildError;
use crate::graph::{Graph, GraphKind};
use crate::ingest::{add_edges, add_nodes, canonicalise_nodes};
use crate::normalize::norm_source_file;

static PERF_LOG: LazyLock<bool> = LazyLock::new(|| std::env::var("GRAPHIFY_PERF_LOG").is_ok());

/// Canonicalise a root path to a string for `source_file` relativisation,
/// falling back to the path as-is when it cannot be resolved (e.g. a
/// non-existent root in tests).
#[must_use]
fn canonicalize_root_to_string(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

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
    let root_str = root.map(canonicalize_root_to_string);
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

/// Load an existing `graph.json`, merge `new_chunks` into it, and return the
/// combined graph. Mirrors graphify-py `build_merge`.
///
/// Never replaces — only grows, except when `prune_sources` is supplied, in
/// which case nodes and edges whose `source_file` matches a pruned (deleted)
/// source path are removed. When `root` is set, absolute `source_file` paths in
/// `new_chunks` are made root-relative.
///
/// # Errors
///
/// - [`BuildError::Security`] if the existing graph file exceeds the size cap.
/// - [`BuildError::WouldShrink`] if the merge would drop nodes without an
///   explicit `prune_sources` opt-in (and `dedup` is off).
/// - Propagates any I/O, JSON, or [`build`] error.
pub fn build_merge(
    new_chunks: &[Value],
    graph_path: &Path,
    prune_sources: Option<&[String]>,
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
) -> Result<Graph, BuildError> {
    build_merge_with_graph_cap(
        new_chunks,
        graph_path,
        prune_sources,
        directed,
        dedup,
        root,
        graphify_security::MAX_GRAPH_FILE_BYTES,
    )
}

/// [`build_merge`] with an explicit graph-file size cap.
///
/// Exposed so callers (and tests) can exercise the oversize-rejection path with
/// a custom cap, mirroring graphify-py's `_MAX_GRAPH_FILE_BYTES` override.
/// Production callers should prefer [`build_merge`].
///
/// # Errors
///
/// See [`build_merge`].
pub fn build_merge_with_graph_cap(
    new_chunks: &[Value],
    graph_path: &Path,
    prune_sources: Option<&[String]>,
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
    graph_cap: u64,
) -> Result<Graph, BuildError> {
    let graph_existed = graph_path.exists();
    let mut all_chunks: Vec<Value> = Vec::with_capacity(new_chunks.len() + 1);
    let mut existing_node_count = 0usize;

    if graph_existed {
        // Read the JSON directly rather than via a graph round-trip: an
        // undirected round-trip re-derives edge endpoints from node-insertion
        // order and silently flips directional edges (#760). The size cap
        // guards against a memory-bomb graph file.
        graphify_security::check_graph_file_size_cap_with(graph_path, graph_cap)?;
        let text = std::fs::read_to_string(graph_path)?;
        let data: Value = serde_json::from_str(&text)?;
        let existing_nodes = data
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let existing_edges = data
            .get("links")
            .or_else(|| data.get("edges"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        existing_node_count = existing_nodes.len();
        all_chunks.push(serde_json::json!({
            "nodes": existing_nodes,
            "edges": existing_edges,
        }));
    }

    all_chunks.extend(new_chunks.iter().cloned());
    let mut graph = build(&all_chunks, directed, dedup, root)?;

    let pruned = prune_sources.unwrap_or(&[]);
    if !pruned.is_empty() {
        prune_deleted_sources(&mut graph, pruned, root);
    }

    // Refuse to silently shrink the graph (#479). Shrinkage is intentional when
    // dedup or prune_sources is active, so only guard otherwise.
    if graph_existed && !dedup && pruned.is_empty() {
        let now = graph.node_count();
        if now < existing_node_count {
            return Err(BuildError::WouldShrink {
                prev: existing_node_count,
                now,
            });
        }
    }

    Ok(graph)
}

/// Remove nodes and edges whose `source_file` matches any deleted source path.
///
/// The match set holds both the raw path (nodes that kept an absolute
/// `source_file`) and its root-relative normalised form, so manifest absolute
/// paths still match nodes relativised at build time (#1007). `.canonicalize()`
/// resolves symlinked roots and redundant `..`/`.` segments.
fn prune_deleted_sources(graph: &mut Graph, pruned: &[String], root: Option<&Path>) {
    let root_str = root.map(canonicalize_root_to_string);
    let mut prune_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in pruned {
        if p.is_empty() {
            continue;
        }
        prune_set.insert(p.clone());
        let norm = norm_source_file(p, root_str.as_deref());
        if !norm.is_empty() {
            prune_set.insert(norm);
        }
    }

    let matches_pruned = |attrs: &indexmap::IndexMap<String, Value>| {
        attrs
            .get("source_file")
            .and_then(Value::as_str)
            .is_some_and(|sf| prune_set.contains(sf))
    };

    let to_remove: Vec<String> = graph
        .nodes()
        .filter(|(_, attrs)| matches_pruned(attrs))
        .map(|(id, _)| id.clone())
        .collect();
    let n_nodes = to_remove.len();
    graph.remove_nodes_from(to_remove.iter().map(String::as_str));
    if n_nodes > 0 {
        eprintln!(
            "[graphify] Pruned {n_nodes} node(s) from {} deleted source file(s).",
            pruned.len()
        );
    }

    let edges_to_remove: Vec<(String, String)> = graph
        .edges()
        .filter(|e| matches_pruned(&e.attrs))
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();
    let n_edges = edges_to_remove.len();
    if n_edges > 0 {
        graph.remove_edges_from(
            edges_to_remove
                .iter()
                .map(|(u, v)| (u.as_str(), v.as_str())),
        );
        eprintln!("[graphify] Pruned {n_edges} edge(s) from deleted source file(s).");
    }

    if n_nodes == 0 && n_edges == 0 {
        eprintln!(
            "[graphify] {} source file(s) deleted since last run — no matching nodes or edges in graph, already clean.",
            pruned.len()
        );
    }
}
