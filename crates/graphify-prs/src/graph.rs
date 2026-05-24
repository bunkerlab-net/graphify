//! Graph-impact analysis: compute which communities a set of changed files
//! touches, and build community labels from graph JSON.

use std::collections::HashMap;
use std::path::Path;

use indexmap::IndexMap;
use serde_json::Value;

use crate::model::path_match;

/// Return `(communities_touched, nodes_affected)` for a set of changed files.
///
/// Mirrors Python's `compute_pr_impact`.  The function accepts the graph data
/// as a pre-built index for efficiency; call [`build_file_index`] first.
#[must_use]
pub fn compute_pr_impact(files: &[String], index: &FileIndex) -> (Vec<i64>, usize) {
    let mut comms: indexmap::IndexSet<i64> = indexmap::IndexSet::new();
    let mut nodes: usize = 0;
    let mut matched: indexmap::IndexSet<String> = indexmap::IndexSet::new();

    for f in files {
        for (src, entry) in &index.0 {
            if !matched.contains(src) && path_match(src, f) {
                comms.extend(entry.communities.iter().copied());
                nodes += entry.node_count;
                matched.insert(src.clone());
            }
        }
    }

    let mut comms_vec: Vec<i64> = comms.into_iter().collect();
    comms_vec.sort_unstable();
    (comms_vec, nodes)
}

/// Per-file entry in the graph index.
struct FileEntry {
    communities: indexmap::IndexSet<i64>,
    node_count: usize,
}

/// Pre-built file → (communities, `node_count`) index.
pub struct FileIndex(HashMap<String, FileEntry>);

/// Build a `FileIndex` from graph JSON node data.
///
/// Accepts the `nodes` array of a graph.json file.
#[must_use]
pub fn build_file_index(nodes: &[Value]) -> FileIndex {
    let mut map: HashMap<String, FileEntry> = HashMap::new();
    for node in nodes {
        let src = node
            .get("source_file")
            .and_then(Value::as_str)
            .unwrap_or("");
        if src.is_empty() {
            continue;
        }
        let entry = map.entry(src.to_string()).or_insert_with(|| FileEntry {
            communities: indexmap::IndexSet::new(),
            node_count: 0,
        });
        if let Some(c) = node.get("community").and_then(Value::as_i64) {
            entry.communities.insert(c);
        }
        entry.node_count += 1;
    }
    FileIndex(map)
}

/// Load `graph.json` from `path`, returning parsed JSON or `None`.
#[must_use]
pub fn load_graph_json(path: &Path) -> Option<Value> {
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Build `{community_id → [top_labels]}` from graph JSON node data.
///
/// Mirrors Python's `build_community_labels`.
#[must_use]
pub fn build_community_labels(data: &Value, top_n: usize) -> IndexMap<i64, Vec<String>> {
    let mut comm_labels: IndexMap<i64, Vec<String>> = IndexMap::new();
    let Some(nodes) = data.get("nodes").and_then(Value::as_array) else {
        return comm_labels;
    };
    for node in nodes {
        let Some(c) = node.get("community").and_then(Value::as_i64) else {
            continue;
        };
        let label = node
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| node.get("id").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        if label.is_empty() {
            continue;
        }
        comm_labels.entry(c).or_default().push(label);
    }
    comm_labels
        .into_iter()
        .map(|(c, labels)| (c, labels.into_iter().take(top_n).collect()))
        .collect()
}
