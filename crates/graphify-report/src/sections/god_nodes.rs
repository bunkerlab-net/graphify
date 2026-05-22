//! God-nodes section renderer.
//!
//! Extracted from `lib.rs` to isolate the "most connected nodes" block.

use serde_json::Value;

/// Render the "God Nodes" section.
pub(crate) fn render_god_nodes(lines: &mut Vec<String>, god_node_list: &[Value]) {
    lines.push(String::new());
    lines.push("## God Nodes (most connected - your core abstractions)".to_string());
    for (i, node) in god_node_list.iter().enumerate() {
        let label = node
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let degree = node.get("degree").and_then(Value::as_u64).unwrap_or(0);
        lines.push(format!("{}. `{label}` - {degree} edges", i + 1));
    }
}
