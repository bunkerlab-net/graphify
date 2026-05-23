//! Convert a raw extraction `serde_json::Value` into nodes and edges on a
//! [`Graph`].

use indexmap::{IndexMap, IndexSet};
use serde_json::Value;

use crate::file_type::coerce_file_type;
use crate::graph::Graph;
use crate::normalize::{norm_source_file, normalize_id};

/// Normalise node objects inside an extraction dict in place.
///
/// Renames `source` → `source_file` and coerces `file_type` values to
/// one of the canonical strings (see [`crate::file_type`]).
pub(crate) fn canonicalise_nodes(extraction: &mut Value) {
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

/// Insert all nodes from an extraction dict into `graph`, normalising
/// `source_file` paths relative to `root_str` if provided.
pub(crate) fn add_nodes(graph: &mut Graph, extraction: &mut Value, root_str: Option<&str>) {
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

/// Resolve a raw edge endpoint string to an exact node ID, falling back
/// to the normalised-ID lookup table when the raw string does not match
/// any existing node verbatim.
fn resolve_edge_id(
    raw: &str,
    node_ids: &IndexSet<String>,
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

/// Insert all edges from an extraction dict into `graph`, resolving
/// endpoint IDs and normalising `source_file` paths.
///
/// Edges whose endpoints cannot be resolved to existing nodes are
/// dropped — this matches the Python reference behaviour for dangling
/// edges (stdlib / external imports).
pub(crate) fn add_edges(graph: &mut Graph, extraction: &Value, root_str: Option<&str>) {
    let Some(edges) = extraction
        .as_object()
        .and_then(|o| o.get("edges"))
        .and_then(Value::as_array)
    else {
        return;
    };
    let node_ids: IndexSet<String> = graph.nodes().map(|(id, _)| id.clone()).collect();
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
