//! Canonicalisation helpers for graph diffing.
//!
//! These functions normalise a graph JSON value so two structurally identical
//! graphs (modulo field ordering, build-time metadata, and community IDs) can
//! be compared with a plain string equality check.
//!
//! Ports `_canonical_graph_for_compare`, `_canonical_topology_for_compare`,
//! `_report_for_compare`, and `_json_text` from
//! `graphify-py/graphify/watch.py:165-271`.

use serde_json::Value;

/// Strip volatile metadata and sort all list keys so two equivalent graphs
/// produce identical JSON strings.
///
/// Ports `_canonical_graph_for_compare`.
///
/// The only field stripped is `built_at_commit`; every list field
/// (`nodes`, `links`, `edges`, `hyperedges`) is sorted by its own JSON
/// representation (matching Python's `json.dumps(item, sort_keys=True)`).
#[must_use]
pub(crate) fn canonical_graph_for_compare(graph_data: &Value) -> Value {
    let Some(obj) = graph_data.as_object() else {
        return graph_data.clone();
    };

    let mut canonical = obj.clone();
    canonical.remove("built_at_commit");

    for key in &["nodes", "links", "edges", "hyperedges"] {
        if let Some(Value::Array(arr)) = canonical.get_mut(*key) {
            let mut sorted = arr.clone();
            sorted.sort_by_key(|item| serde_json::to_string(item).unwrap_or_default());
            *arr = sorted;
        }
    }

    Value::Object(canonical)
}

/// Strip community IDs and `_src`/`_tgt` fields, then sort lists for topology
/// comparison.
///
/// Ports `_canonical_topology_for_compare`.
///
/// Use this when you want to know whether the *structure* of the graph changed
/// (ignoring re-assigned community IDs from Louvain's non-determinism).
#[must_use]
pub(crate) fn canonical_topology_for_compare(graph_data: &Value) -> Value {
    let Some(obj) = graph_data.as_object() else {
        return graph_data.clone();
    };

    let mut canonical = obj.clone();
    canonical.remove("built_at_commit");

    // Normalise nodes: strip community + norm_label, then sort.
    if let Some(Value::Array(nodes)) = canonical.get_mut("nodes") {
        let mut norm_nodes: Vec<Value> = nodes
            .iter()
            .filter_map(|node| {
                let map = node.as_object()?;
                let mut n = map.clone();
                n.remove("community");
                n.remove("norm_label");
                Some(Value::Object(n))
            })
            .collect();
        norm_nodes.sort_by_key(|item| serde_json::to_string(item).unwrap_or_default());
        *nodes = norm_nodes;
    }

    // Normalise edges and links: strip _src/_tgt (remapping to source/target),
    // drop confidence_score, then sort.
    for key in &["links", "edges"] {
        if let Some(Value::Array(edges)) = canonical.get_mut(*key) {
            let mut norm_edges: Vec<Value> = edges
                .iter()
                .filter_map(|edge| {
                    let map = edge.as_object()?;
                    let mut e = map.clone();
                    let true_src = e.remove("_src");
                    let true_tgt = e.remove("_tgt");
                    if let (Some(s), Some(t)) = (true_src, true_tgt) {
                        e.insert("source".to_string(), s);
                        e.insert("target".to_string(), t);
                    }
                    e.remove("confidence_score");
                    Some(Value::Object(e))
                })
                .collect();
            norm_edges.sort_by_key(|item| serde_json::to_string(item).unwrap_or_default());
            *edges = norm_edges;
        }
    }

    // Sort hyperedges.
    if let Some(Value::Array(hyper)) = canonical.get_mut("hyperedges") {
        hyper.sort_by_key(|item| serde_json::to_string(item).unwrap_or_default());
    }

    Value::Object(canonical)
}

/// Strip the "Built from commit" line from a report before comparison.
///
/// Ports `_report_for_compare`.
///
/// This ensures that a rebuild at the same commit doesn't look different from
/// one done at a prior commit when the graph itself hasn't changed.
#[must_use]
pub(crate) fn report_for_compare(report_text: &str) -> String {
    // Remove lines of the form: `- Built from commit: `<hash>``
    let mut result = String::with_capacity(report_text.len());
    for line in report_text.lines() {
        let trimmed = line.trim_start_matches('-').trim();
        if trimmed.starts_with("Built from commit:") {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    // Preserve a trailing newline if the input didn't end with one.
    if !report_text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Serialise a JSON value to a pretty-printed string with a trailing newline.
///
/// Ports `_json_text`.
///
/// # Errors
///
/// Returns an empty string on serialisation failure (which should never occur
/// for well-formed `serde_json::Value` values).
#[must_use]
pub(crate) fn json_text(data: &Value) -> String {
    match serde_json::to_string_pretty(data) {
        Ok(s) => s + "\n",
        Err(_) => String::new(),
    }
}
