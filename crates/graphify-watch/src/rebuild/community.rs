//! Node-to-community mapping helper.
//!
//! Extracted from `rebuild.rs` so the community-map construction that bridges
//! the serialised graph JSON and the clustering output lives in isolation.

use indexmap::IndexMap;
use serde_json::Value;

/// Build a `{node_id → community_id}` map from a serialised graph JSON value.
///
/// Nodes with a missing or non-numeric `community` field are skipped with a
/// warning, matching Python's behaviour.
///
/// Ports `_node_community_map` from `watch.py:146-162`.
#[must_use]
pub fn node_community_map(graph_data: &Value) -> IndexMap<String, i64> {
    let mut out = IndexMap::new();
    let Some(nodes) = graph_data.get("nodes").and_then(Value::as_array) else {
        return out;
    };

    for node in nodes {
        let node_id = match node.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };
        let community = node.get("community");
        match community {
            Some(Value::Number(n)) => {
                if let Some(cid) = n.as_i64() {
                    out.insert(node_id, cid);
                } else {
                    eprintln!(
                        "[graphify watch] Skipping node with invalid community id: \
                         node_id={node_id:?} community={n:?}"
                    );
                }
            }
            None => {}
            Some(other) => {
                eprintln!(
                    "[graphify watch] Skipping node with invalid community id: \
                     node_id={node_id:?} community={other:?}"
                );
            }
        }
    }
    out
}
