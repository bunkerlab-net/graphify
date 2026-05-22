//! Graph assembly from extraction dicts.
//!
//! Ports `graphify-py/graphify/build.py`. Provides a [`Graph`] type that
//! mirrors `NetworkX` `Graph` / `DiGraph` / `MultiGraph` / `MultiDiGraph`
//! semantics closely enough for byte-identical JSON round-trips.

use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

mod normalize;
pub use normalize::{norm_source_file, normalize_id};

mod graph;
pub use graph::{Edge, Graph, GraphKind};

mod dedup_label;
pub use dedup_label::deduplicate_by_label;

/// Build-layer errors.
#[derive(Debug, Error)]
pub enum BuildError {
    #[error(
        "graphify: build_merge would shrink graph from {prev} → {now} nodes. Pass prune_sources explicitly if you intend to remove nodes."
    )]
    WouldShrink { prev: usize, now: usize },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Map of known invalid `file_type` values that LLM subagents commonly emit.
fn file_type_synonym(s: &str) -> Option<&'static str> {
    match s {
        "markdown" | "text" => Some("document"),
        "tool" | "library" => Some("code"),
        "pattern" | "principle" | "constraint" | "tech" | "technology" | "data-source"
        | "data_source" | "gotcha" | "framework" => Some("concept"),
        _ => None,
    }
}

const VALID_FILE_TYPES: &[&str] = &["code", "document", "paper", "image", "rationale", "concept"];

fn coerce_file_type(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => {
            if s.is_empty() {
                Some("concept".to_string())
            } else if VALID_FILE_TYPES.contains(&s.as_str()) {
                None
            } else {
                Some(file_type_synonym(s).unwrap_or("concept").to_string())
            }
        }
        _ => Some("concept".to_string()),
    }
}

fn canonicalise_nodes(extraction: &mut Value) {
    let Some(nodes) = extraction
        .as_object_mut()
        .and_then(|o| o.get_mut("nodes"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for node in nodes.iter_mut() {
        let Some(map) = node.as_object_mut() else {
            continue;
        };
        if map.contains_key("source") && !map.contains_key("source_file") {
            let src = map.remove("source").unwrap_or(Value::Null);
            map.insert("source_file".to_string(), src);
        }
        if let Some(new_ft) = coerce_file_type(map.get("file_type")) {
            map.insert("file_type".to_string(), Value::String(new_ft));
        }
    }
}

fn add_nodes(graph: &mut Graph, extraction: &mut Value, root_str: Option<&str>) {
    let Some(nodes) = extraction
        .as_object_mut()
        .and_then(|o| o.get_mut("nodes"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for node in nodes.iter_mut() {
        let Some(map) = node.as_object_mut() else {
            continue;
        };
        let Some(id) = map.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(Value::String(sf)) = map.get_mut("source_file") {
            *sf = norm_source_file(sf, root_str);
        }
        let mut attrs: IndexMap<String, Value> = IndexMap::new();
        for (k, v) in &*map {
            if k == "id" {
                continue;
            }
            attrs.insert(k.clone(), v.clone());
        }
        graph.add_node(&id, attrs);
    }
}

fn resolve_edge_id(
    raw: &str,
    node_ids: &indexmap::IndexSet<String>,
    norm_to_id: &IndexMap<String, String>,
) -> String {
    if node_ids.contains(raw) {
        return raw.to_string();
    }
    norm_to_id
        .get(&normalize_id(raw))
        .cloned()
        .unwrap_or_else(|| raw.to_string())
}

fn add_edges(graph: &mut Graph, extraction: &Value, root_str: Option<&str>) {
    let Some(edges) = extraction
        .as_object()
        .and_then(|o| o.get("edges"))
        .and_then(Value::as_array)
    else {
        return;
    };
    let node_ids: indexmap::IndexSet<String> = graph.nodes().map(|(id, _)| id.clone()).collect();
    let norm_to_id: IndexMap<String, String> = node_ids
        .iter()
        .map(|nid| (normalize_id(nid), nid.clone()))
        .collect();

    for edge in edges {
        let Some(orig) = edge.as_object() else {
            continue;
        };
        let mut map = orig.clone();
        if !map.contains_key("source")
            && let Some(v) = map.remove("from")
        {
            map.insert("source".to_string(), v);
        }
        if !map.contains_key("target")
            && let Some(v) = map.remove("to")
        {
            map.insert("target".to_string(), v);
        }
        let Some(src) = map.get("source").and_then(Value::as_str) else {
            continue;
        };
        let Some(tgt) = map.get("target").and_then(Value::as_str) else {
            continue;
        };
        let resolved_src = resolve_edge_id(src, &node_ids, &norm_to_id);
        let resolved_tgt = resolve_edge_id(tgt, &node_ids, &norm_to_id);
        if !node_ids.contains(&resolved_src) || !node_ids.contains(&resolved_tgt) {
            continue;
        }
        let mut attrs: IndexMap<String, Value> = IndexMap::new();
        for (k, v) in &map {
            if k == "source" || k == "target" {
                continue;
            }
            if k == "source_file"
                && let Value::String(sf) = v
            {
                attrs.insert(k.clone(), Value::String(norm_source_file(sf, root_str)));
                continue;
            }
            attrs.insert(k.clone(), v.clone());
        }
        attrs.insert("_src".to_string(), Value::String(resolved_src.clone()));
        attrs.insert("_tgt".to_string(), Value::String(resolved_tgt.clone()));
        graph.add_edge(&resolved_src, &resolved_tgt, attrs);
    }
}

/// Build a graph from a single extraction dict.
///
/// Mirrors Python `build_from_json(extraction, directed=False, root=None)`.
///
/// # Errors
///
/// Currently infallible (returns `Result` for API parity with the future
/// `build_merge` which can fail with [`BuildError::WouldShrink`]).
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

    canonicalise_nodes(&mut extraction);
    let _ = graphify_validate::validate_extraction(&extraction);

    let mut graph = Graph::new(kind);
    add_nodes(&mut graph, &mut extraction, root_str.as_deref());
    add_edges(&mut graph, &extraction, root_str.as_deref());

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

/// Merge multiple extraction dicts into one graph. Mirrors Python `build(...)`.
///
/// `dedup` runs entity deduplication via [`deduplicate_by_label`]. The Python
/// version optionally also calls `graphify.dedup.deduplicate_entities` for
/// LLM-assisted fuzzy matching — that path requires the `graphify-dedup` crate
/// and is opt-in via `dedup_llm_backend`. For now we only support the cheap
/// label-canonical dedup path; LLM-backed dedup is reserved for future work.
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

/// Rewrite every node ID to be prefixed with `repo_tag::`, preserving labels.
#[must_use]
pub fn prefix_graph_for_global(graph: &Graph, repo_tag: &str) -> Graph {
    let mut relabel: IndexMap<String, String> = IndexMap::new();
    for (id, _) in graph.nodes() {
        relabel.insert(id.clone(), format!("{repo_tag}::{id}"));
    }
    let mut out = graph.clone();
    out.relabel_nodes(&relabel);
    for (id, attrs) in out.nodes_mut() {
        attrs.insert("repo".to_string(), Value::String(repo_tag.to_string()));
        let local = id
            .split_once("::")
            .map_or(id.clone(), |(_, l)| l.to_string());
        attrs
            .entry("local_id".to_string())
            .or_insert(Value::String(local));
    }
    out
}

/// Remove every node tagged with `repo_tag` in place. Returns the count removed.
pub fn prune_repo_from_graph(graph: &mut Graph, repo_tag: &str) -> usize {
    let to_remove: Vec<String> = graph
        .nodes()
        .filter(|(_, attrs)| attrs.get("repo").and_then(Value::as_str) == Some(repo_tag))
        .map(|(id, _)| id.clone())
        .collect();
    let n = to_remove.len();
    graph.remove_nodes_from(to_remove.iter().map(String::as_str));
    n
}

// ---------------------------------------------------------------------------
// Compat with the previous stub: keep these typedefs around so other crates
// that imported them still compile.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeAttrs {
    pub id: String,
    pub label: String,
    pub file_type: String,
    pub source_file: String,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EdgeAttrs {
    pub relation: String,
    pub confidence: String,
    pub source_file: String,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}
