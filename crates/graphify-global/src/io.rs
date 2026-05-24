//! Global-graph file I/O in the `NetworkX` `node_link_data` JSON shape.

use std::path::Path;

use serde_json::Value;

use graphify_build::{Graph, GraphKind};

use crate::error::GlobalError;

/// Load a [`Graph`] from a NetworkX-style `node_link_data` JSON file.
///
/// Normalises `"links"` → `"edges"` before passing to
/// [`graphify_build::build_from_json`] so both key spellings are accepted
/// (`NetworkX` writes `"links"` by default; older graphify dumps used
/// `"edges"`).
///
/// Returns an empty graph if `path` does not exist.
///
/// # Errors
///
/// Returns [`GlobalError::Io`] or [`GlobalError::Json`] if the file
/// cannot be read or parsed, or [`GlobalError::Build`] if the graph
/// builder rejects the JSON shape.
pub fn load_graph_from_file(path: &Path) -> Result<Graph, GlobalError> {
    if !path.exists() {
        return Ok(Graph::new(GraphKind::Graph));
    }
    graphify_security::check_graph_file_size_cap(path)?;
    let text = std::fs::read_to_string(path)?;
    let mut data: serde_json::Map<String, Value> = serde_json::from_str(&text)?;

    if !data.contains_key("edges")
        && let Some(links) = data.remove("links")
    {
        data.insert("edges".to_string(), links);
    }

    let directed = data
        .get("directed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let extraction = Value::Object(data);
    graphify_build::build_from_json(extraction, directed, None)
        .map_err(|e| GlobalError::Build(e.to_string()))
}

/// Serialise a [`Graph`] to the `NetworkX` `node_link_data` JSON format
/// and write it to `path`.
///
/// Always emits `"links"` (not `"edges"`) for round-trip compatibility
/// with `nx.node_link_graph` callers.
///
/// # Errors
///
/// Returns [`GlobalError::Io`] or [`GlobalError::Json`] on write failure.
pub fn save_graph_to_file(path: &Path, graph: &Graph) -> Result<(), GlobalError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let nodes: Vec<Value> = graph
        .nodes()
        .map(|(id, attrs)| {
            let mut obj = serde_json::Map::new();
            obj.insert("id".to_string(), Value::String(id.clone()));
            for (k, v) in attrs {
                obj.insert(k.clone(), v.clone());
            }
            Value::Object(obj)
        })
        .collect();

    let links: Vec<Value> = graph
        .edges()
        .map(|edge| {
            let mut obj = serde_json::Map::new();
            obj.insert("source".to_string(), Value::String(edge.source.clone()));
            obj.insert("target".to_string(), Value::String(edge.target.clone()));
            for (k, v) in &edge.attrs {
                if k != "_src" && k != "_tgt" {
                    obj.insert(k.clone(), v.clone());
                }
            }
            Value::Object(obj)
        })
        .collect();

    let payload = serde_json::json!({
        "directed": graph.kind.is_directed(),
        "multigraph": graph.kind.is_multi(),
        "graph": {},
        "nodes": nodes,
        "links": links,
    });

    let text = serde_json::to_string_pretty(&payload)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Return the first 16 hex characters of the SHA-256 digest of `path`'s
/// contents. Mirrors Python `_file_hash(path)`.
///
/// # Errors
///
/// Returns [`GlobalError::Io`] if the file cannot be read.
pub fn file_hash(path: &Path) -> Result<String, GlobalError> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(hex::encode(&digest[..8])) // 8 bytes → 16 hex chars
}
