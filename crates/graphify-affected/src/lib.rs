//! Reverse-traversal impact analysis: given a seed node, enumerate every
//! node that depends on it via configurable edge relations up to a given
//! depth.
//!
//! Ports `graphify-py/graphify/affected.py`. Used by the `graphify
//! affected` CLI subcommand to answer "if I change X, what else is
//! affected?" — a fast pre-flight before refactors and bulk edits.

use std::collections::VecDeque;
use std::path::Path;

use indexmap::{IndexMap, IndexSet};
use serde_json::Value;
use thiserror::Error;

use graphify_build::{Graph, build_from_json};

/// Default edge relations followed during reverse traversal. Matches
/// `DEFAULT_AFFECTED_RELATIONS` in the Python source.
pub const DEFAULT_AFFECTED_RELATIONS: &[&str] = &[
    "calls",
    "references",
    "imports",
    "imports_from",
    "re_exports",
    "inherits",
    "extends",
    "implements",
    "uses",
    "mixes_in",
    "embeds",
];

/// A node that depends, directly or transitively, on the seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedHit {
    /// The dependent node's ID.
    pub node_id: String,
    /// Distance (in edge hops) from the seed.
    pub depth: usize,
    /// Relation used to reach this node from its successor.
    pub via_relation: String,
}

/// Errors raised by the affected-traversal pipeline.
#[derive(Debug, Error)]
pub enum AffectedError {
    /// Underlying filesystem I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON parse error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Graph file failed the memory-bomb size cap.
    #[error(transparent)]
    Security(#[from] graphify_security::SecurityError),

    /// Build layer failed to assemble the graph.
    #[error("build error: {0}")]
    Build(String),
}

/// Resolve a free-form query string to a node ID using a fuzzy fallback
/// chain. Returns `None` when the query is ambiguous or has no match.
///
/// Resolution order (matching the Python `resolve_seed`):
/// 1. Exact node-ID match.
/// 2. Exact label match (case-insensitive). Skipped when there is more
///    than one matching node.
/// 3. Exact `source_file` match (case-insensitive). Skipped when more
///    than one matches.
/// 4. Single substring (case-insensitive) match on label.
#[must_use]
pub fn resolve_seed(graph: &Graph, query: &str) -> Option<String> {
    if graph.node_data(query).is_some() {
        return Some(query.to_string());
    }
    let q = query.to_lowercase();

    let exact_label_matches: Vec<String> = graph
        .nodes()
        .filter(|(_, data)| {
            data.get("label")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
                == Some(q.clone())
        })
        .map(|(id, _)| id.clone())
        .collect();
    if exact_label_matches.len() == 1 {
        return exact_label_matches.into_iter().next();
    }

    let exact_source_matches: Vec<String> = graph
        .nodes()
        .filter(|(_, data)| {
            data.get("source_file")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
                == Some(q.clone())
        })
        .map(|(id, _)| id.clone())
        .collect();
    if exact_source_matches.len() == 1 {
        return exact_source_matches.into_iter().next();
    }

    let contains_matches: Vec<String> = graph
        .nodes()
        .filter(|(_, data)| {
            data.get("label")
                .and_then(Value::as_str)
                .is_some_and(|label| label.to_lowercase().contains(&q))
        })
        .map(|(id, _)| id.clone())
        .collect();
    if contains_matches.len() == 1 {
        return contains_matches.into_iter().next();
    }
    None
}

/// Reverse-BFS from `seed` along edges whose `relation` is in
/// `relations`, up to `depth` hops. Returns nodes in BFS visit order.
///
/// Mirrors `affected_nodes` in Python. Self-edges and revisits are
/// silently skipped.
#[allow(clippy::similar_names)] // `seed` / `seen` are domain-canonical names.
#[must_use]
pub fn affected_nodes(
    graph: &Graph,
    seed: &str,
    relations: &[&str],
    depth: usize,
) -> Vec<AffectedHit> {
    let relation_set: IndexSet<&str> = relations.iter().copied().collect();
    let in_edges = build_in_edges(graph);

    let mut seen: IndexSet<String> = IndexSet::new();
    seen.insert(seed.to_string());
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((seed.to_string(), 0));
    let mut hits: Vec<AffectedHit> = Vec::new();

    while let Some((current, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }
        let Some(incoming) = in_edges.get(&current) else {
            continue;
        };
        for (src, relation) in incoming {
            if !relation_set.contains(relation.as_str()) {
                continue;
            }
            if seen.contains(src) {
                continue;
            }
            seen.insert(src.clone());
            hits.push(AffectedHit {
                node_id: src.clone(),
                depth: current_depth + 1,
                via_relation: relation.clone(),
            });
            queue.push_back((src.clone(), current_depth + 1));
        }
    }

    hits
}

/// Render an `affected` query as a human-readable report.
///
/// Format matches the Python `format_affected` line-by-line so existing
/// scripts that grep its output continue to work.
#[must_use]
pub fn format_affected(graph: &Graph, query: &str, relations: &[&str], depth: usize) -> String {
    let Some(seed) = resolve_seed(graph, query) else {
        return format!("No unique node match for {query}");
    };

    let hits = affected_nodes(graph, &seed, relations, depth);
    let label = node_label(graph, &seed);
    let mut lines: Vec<String> = vec![
        format!("Affected nodes for {label}"),
        format!("Relations: {}", relations.join(", ")),
        format!("Depth: {depth}"),
    ];
    if hits.is_empty() {
        lines.push("No affected nodes found.".to_string());
        return lines.join("\n");
    }
    for hit in &hits {
        let label = node_label(graph, &hit.node_id);
        let data = graph.node_data(&hit.node_id);
        let loc = format_location(data);
        lines.push(format!("- {label} [{}] {loc}", hit.via_relation));
    }
    lines.join("\n")
}

/// Load a graph JSON file (either `links` or `edges` shape) into a
/// [`Graph`]. Applies the memory-bomb size cap before reading.
///
/// # Errors
///
/// Returns [`AffectedError::Security`] when the file exceeds the cap;
/// [`AffectedError::Io`] / [`AffectedError::Json`] on read or parse
/// failure; [`AffectedError::Build`] when `build_from_json` rejects the
/// shape.
pub fn load_graph(path: &Path) -> Result<Graph, AffectedError> {
    graphify_security::check_graph_file_size_cap(path)?;
    let text = std::fs::read_to_string(path)?;
    let mut data: Value = serde_json::from_str(&text)?;
    if let Some(obj) = data.as_object_mut()
        && !obj.contains_key("edges")
        && let Some(links) = obj.remove("links")
    {
        obj.insert("edges".to_string(), links);
    }
    let directed = data
        .as_object()
        .and_then(|o| o.get("directed"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    build_from_json(data, directed, None).map_err(|e| AffectedError::Build(e.to_string()))
}

/// Build the `node_id → Vec<(source_id, relation)>` incoming-edge index.
fn build_in_edges(graph: &Graph) -> IndexMap<String, Vec<(String, String)>> {
    let mut in_edges: IndexMap<String, Vec<(String, String)>> = IndexMap::new();
    for edge in graph.edges() {
        let relation = edge
            .attrs
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        in_edges
            .entry(edge.target.clone())
            .or_default()
            .push((edge.source.clone(), relation));
    }
    in_edges
}

fn node_label(graph: &Graph, node_id: &str) -> String {
    graph
        .node_data(node_id)
        .and_then(|d| d.get("label"))
        .and_then(Value::as_str)
        .map_or_else(|| node_id.to_owned(), str::to_owned)
}

fn format_location(data: Option<&IndexMap<String, Value>>) -> String {
    let Some(d) = data else {
        return "-".to_string();
    };
    let source_file = d
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    let source_location = d.get("source_location").and_then(Value::as_str);
    match source_location {
        Some(loc) if !loc.is_empty() => format!("{source_file}:{loc}"),
        _ => source_file,
    }
}
