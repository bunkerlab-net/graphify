//! Operations that prepare per-repo graphs for the global cross-corpus
//! graph and that remove a repo's contribution from a global graph.

use indexmap::IndexMap;
use serde_json::Value;

use crate::graph::Graph;

/// Rewrite every node ID to be prefixed with `repo_tag::`, preserving
/// labels.
///
/// Also annotates each node with `repo` and `local_id` so callers can
/// recover the original ID and the source repo.
#[must_use]
pub fn prefix_graph_for_global(graph: &Graph, repo_tag: &str) -> Graph {
    let mut relabel: IndexMap<String, String> = IndexMap::new();
    for (id, _) in graph.nodes() {
        relabel.insert(id.clone(), format!("{repo_tag}::{id}"));
    }
    let mut out = graph.clone();
    out.relabel_nodes(&relabel);
    for (id, attrs) in out.nodes_mut() {
        // `repo` must always reflect the current prefix; `local_id` is
        // preserved if a prior `prefix_graph_for_global` call already set
        // it so the original (pre-prefix) ID is never lost on re-prefix.
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

/// Remove every node tagged with `repo_tag` in place. Returns the count
/// removed.
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
