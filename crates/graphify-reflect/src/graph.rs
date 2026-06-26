//! Optional graph artifacts: community grouping and the node-existence gate.
//!
//! Mirrors how `graphify export wiki` reads `graph.json` +
//! `.graphify_analysis.json` + `.graphify_labels.json`. Community membership in
//! the analysis sidecar is keyed by node id, but `save-result` cites nodes by
//! label, so both id and label are mapped to a community. Best-effort: any
//! missing/unparseable artifact disables grouping.

use std::collections::HashSet;
use std::path::Path;

use indexmap::IndexMap;
use serde_json::Value;

use crate::UNCATEGORIZED;

/// Read and parse a JSON file, or `None` on any I/O or parse failure.
fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Build a lookup from node id AND node label → community label, or `None` if
/// the graph isn't available. Label collisions resolve to the smallest community
/// id (sorted-cid iteration + first-write-wins).
#[must_use]
pub fn load_node_community(
    graph_path: &Path,
    analysis_path: &Path,
    labels_path: &Path,
) -> Option<IndexMap<String, String>> {
    if !graph_path.exists() || !analysis_path.exists() {
        return None;
    }
    let analysis = read_json(analysis_path)?;
    let communities = analysis.get("communities").and_then(Value::as_object)?;
    if communities.is_empty() {
        return None;
    }
    let labels = read_json(labels_path)
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    // id -> label from the graph, so a label-form citation resolves too.
    let mut id_to_label: IndexMap<String, String> = IndexMap::new();
    if let Some(nodes) = read_json(graph_path)
        .as_ref()
        .and_then(|g| g.get("nodes"))
        .and_then(Value::as_array)
    {
        for n in nodes {
            if let (Some(id), Some(label)) = (n.get("id"), n.get("label"))
                && !id.is_null()
                && !label.is_null()
            {
                id_to_label.insert(json_to_string(id), json_to_string(label));
            }
        }
    }

    // Sorted cid iteration + first-write-wins makes any collision deterministic.
    let mut cids: Vec<&String> = communities.keys().collect();
    cids.sort();
    let mut node_community: IndexMap<String, String> = IndexMap::new();
    for cid in cids {
        let label = labels
            .get(cid)
            .and_then(Value::as_str)
            .map_or_else(|| format!("Community {cid}"), str::to_string);
        let Some(members) = communities.get(cid).and_then(Value::as_array) else {
            continue;
        };
        for member in members {
            let nid = json_to_string(member);
            node_community
                .entry(nid.clone())
                .or_insert_with(|| label.clone());
            if let Some(nlabel) = id_to_label.get(&nid) {
                node_community
                    .entry(nlabel.clone())
                    .or_insert_with(|| label.clone());
            }
        }
    }
    Some(node_community)
}

/// The set of node ids AND labels in the current graph, or `None` if
/// unavailable. Used to drop source nodes whose code is gone.
#[must_use]
pub fn load_known_nodes(graph_path: &Path) -> Option<HashSet<String>> {
    let nodes = read_json(graph_path)?
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()?;
    let mut known: HashSet<String> = HashSet::new();
    for n in &nodes {
        if let Some(id) = n.get("id").filter(|v| !v.is_null()) {
            known.insert(json_to_string(id));
        }
        if let Some(label) = n.get("label").filter(|v| !v.is_null()) {
            known.insert(json_to_string(label));
        }
    }
    if known.is_empty() { None } else { Some(known) }
}

/// The community a doc belongs to: the plurality community of its source nodes,
/// ties broken to the lexicographically-smallest label. Docs with no resolvable
/// community fall into the `Uncategorized` bucket.
#[must_use]
pub(crate) fn doc_community(
    nodes: &[String],
    node_community: Option<&IndexMap<String, String>>,
) -> String {
    let Some(nc) = node_community.filter(|m| !m.is_empty()) else {
        return UNCATEGORIZED.to_string();
    };
    let mut counts: IndexMap<&str, usize> = IndexMap::new();
    for n in nodes {
        if let Some(label) = nc.get(n) {
            *counts.entry(label.as_str()).or_insert(0) += 1;
        }
    }
    if counts.is_empty() {
        return UNCATEGORIZED.to_string();
    }
    // min over (-count, label): highest count wins, then smallest label.
    counts
        .iter()
        .min_by(|a, b| (std::cmp::Reverse(*a.1), *a.0).cmp(&(std::cmp::Reverse(*b.1), *b.0)))
        .map_or_else(
            || UNCATEGORIZED.to_string(),
            |(label, _)| (*label).to_string(),
        )
}

/// Stringify a JSON scalar the way Python's `str(node_id)` would for the
/// id/label forms `save-result` and the graph use (strings stay verbatim).
fn json_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
