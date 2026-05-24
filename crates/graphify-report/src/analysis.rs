//! Extract structured pieces of the `analysis` JSON object passed to
//! [`crate::render_report`].

use std::collections::HashMap;

use serde_json::Value;

/// Alias for the community membership list: `[(community_id, [node_id, ...])]`.
pub(crate) type Communities<'a> = Vec<(i64, Vec<&'a str>)>;

/// Extract the community membership list from the analysis object.
///
/// Returns an empty list if `communities` is missing or malformed.
pub(crate) fn extract_communities(obj: &serde_json::Map<String, Value>) -> Communities<'_> {
    obj.get("communities")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    let cid = k.parse::<i64>().ok()?;
                    let nodes = v.as_array()?.iter().filter_map(Value::as_str).collect();
                    Some((cid, nodes))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Extract per-community cohesion scores from the analysis object.
pub(crate) fn extract_cohesion(obj: &serde_json::Map<String, Value>) -> HashMap<i64, f64> {
    obj.get("cohesion_scores")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.parse::<i64>().ok()?, v.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// Extract per-community display labels from the analysis object.
pub(crate) fn extract_labels(obj: &serde_json::Map<String, Value>) -> HashMap<i64, &str> {
    obj.get("community_labels")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.parse::<i64>().ok()?, v.as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}
