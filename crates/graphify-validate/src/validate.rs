//! The [`validate_extraction`] and [`assert_valid`] entry points.

use std::collections::HashSet;

use serde_json::Value;

use crate::error::ValidationError;
use crate::schema::{
    REQUIRED_EDGE_FIELDS, REQUIRED_NODE_FIELDS, VALID_CONFIDENCES, VALID_FILE_TYPES,
};

/// Return a sorted `Vec<String>` copy of a string-slice list.
fn sorted(set: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = set.iter().map(|s| (*s).to_string()).collect();
    v.sort();
    v
}

/// Format a JSON `Value` as a human-readable repr, wrapping strings in
/// single quotes so they round-trip the same as Python's `repr()`.
fn repr(value: &Value) -> String {
    match value {
        Value::String(s) => format!("'{s}'"),
        other => other.to_string(),
    }
}

/// Validate an extraction JSON value against the graphify schema.
///
/// Returns a list of error strings — empty means valid. Mirrors the
/// Python `validate_extraction` function byte-for-byte where practical.
#[must_use]
pub fn validate_extraction(data: &Value) -> Vec<String> {
    let Some(obj) = data.as_object() else {
        return vec!["Extraction must be a JSON object".to_string()];
    };

    let mut errors: Vec<String> = Vec::new();

    let nodes_value = obj.get("nodes");
    let mut node_ids: HashSet<String> = HashSet::new();
    match nodes_value {
        None => errors.push("Missing required key 'nodes'".to_string()),
        Some(Value::Array(nodes)) => {
            for (i, node) in nodes.iter().enumerate() {
                let Some(node_obj) = node.as_object() else {
                    errors.push(format!("Node {i} must be an object"));
                    continue;
                };
                let id_repr = node_obj.get("id").map_or_else(|| "'?'".to_string(), repr);
                for field in REQUIRED_NODE_FIELDS {
                    if !node_obj.contains_key(*field) {
                        errors.push(format!(
                            "Node {i} (id={id_repr}) missing required field '{field}'"
                        ));
                    }
                }
                if let Some(ft) = node_obj.get("file_type").and_then(Value::as_str)
                    && !VALID_FILE_TYPES.contains(&ft)
                {
                    let allowed = sorted(VALID_FILE_TYPES);
                    errors.push(format!(
                            "Node {i} (id={id_repr}) has invalid file_type '{ft}' - must be one of {allowed:?}"
                        ));
                }
                if let Some(Value::String(id)) = node_obj.get("id") {
                    node_ids.insert(id.clone());
                }
            }
        }
        Some(_) => errors.push("'nodes' must be a list".to_string()),
    }

    // Edges — accept "links" (NetworkX <= 3.1) as fallback.
    let edge_list = obj.get("edges").or_else(|| obj.get("links"));
    match edge_list {
        None => errors.push("Missing required key 'edges'".to_string()),
        Some(Value::Array(edges)) => {
            for (i, edge) in edges.iter().enumerate() {
                let Some(edge_obj) = edge.as_object() else {
                    errors.push(format!("Edge {i} must be an object"));
                    continue;
                };
                for field in REQUIRED_EDGE_FIELDS {
                    if !edge_obj.contains_key(*field) {
                        errors.push(format!("Edge {i} missing required field '{field}'"));
                    }
                }
                if let Some(conf) = edge_obj.get("confidence").and_then(Value::as_str)
                    && !VALID_CONFIDENCES.contains(&conf)
                {
                    let allowed = sorted(VALID_CONFIDENCES);
                    errors.push(format!(
                        "Edge {i} has invalid confidence '{conf}' - must be one of {allowed:?}"
                    ));
                }
                if !node_ids.is_empty() {
                    if let Some(src) = edge_obj.get("source").and_then(Value::as_str)
                        && !node_ids.contains(src)
                    {
                        errors.push(format!(
                            "Edge {i} source '{src}' does not match any node id"
                        ));
                    }
                    if let Some(tgt) = edge_obj.get("target").and_then(Value::as_str)
                        && !node_ids.contains(tgt)
                    {
                        errors.push(format!(
                            "Edge {i} target '{tgt}' does not match any node id"
                        ));
                    }
                }
            }
        }
        Some(_) => errors.push("'edges' must be a list".to_string()),
    }

    errors
}

/// Raise [`ValidationError`] with all errors if extraction is invalid.
///
/// Mirrors Python `assert_valid`.
///
/// # Errors
///
/// Returns [`ValidationError`] with the same error messages
/// [`validate_extraction`] would produce when the data violates the
/// schema.
pub fn assert_valid(data: &Value) -> Result<(), ValidationError> {
    let errors = validate_extraction(data);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationError { errors })
    }
}
