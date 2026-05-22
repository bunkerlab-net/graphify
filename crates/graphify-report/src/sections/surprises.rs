//! Surprising-connections and hyperedges section renderers.
//!
//! Extracted from `lib.rs` to group the two cross-community relationship
//! renderers together.

use serde_json::Value;

/// Render the "Surprising Connections" section.
pub(crate) fn render_surprising(lines: &mut Vec<String>, surprise_list: &[Value]) {
    lines.push(String::new());
    lines.push("## Surprising Connections (you probably didn't know these)".to_string());
    if surprise_list.is_empty() {
        lines.push(
            "- None detected - all connections are within the same source files.".to_string(),
        );
        return;
    }
    for s in surprise_list {
        let source = s.get("source").and_then(Value::as_str).unwrap_or_default();
        let target = s.get("target").and_then(Value::as_str).unwrap_or_default();
        let relation = s
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("related_to");
        let note = s.get("note").and_then(Value::as_str).unwrap_or_default();
        let src_files = s.get("source_files").and_then(Value::as_array);
        let src0 = src_files
            .and_then(|f| f.first())
            .and_then(Value::as_str)
            .unwrap_or_default();
        let src1 = src_files
            .and_then(|f| f.get(1))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let conf = s
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("EXTRACTED");
        let cscore = s.get("confidence_score").and_then(Value::as_f64);
        let conf_tag = if conf == "INFERRED" {
            if let Some(cs) = cscore {
                format!("INFERRED {cs:.2}")
            } else {
                conf.to_string()
            }
        } else {
            conf.to_string()
        };
        let sem_tag = if relation == "semantically_similar_to" {
            " [semantically similar]"
        } else {
            ""
        };
        lines.push(format!(
            "- `{source}` --{relation}--> `{target}`  [{conf_tag}]{sem_tag}"
        ));
        let note_part = if note.is_empty() {
            String::new()
        } else {
            format!("  _{note}_")
        };
        lines.push(format!("  {src0} → {src1}{note_part}"));
    }
}

/// Render the "Hyperedges" section when hyperedge data is present.
pub(crate) fn render_hyperedges(lines: &mut Vec<String>, hyperedges: &[Value]) {
    lines.push(String::new());
    lines.push("## Hyperedges (group relationships)".to_string());
    for h in hyperedges {
        let node_labels = h
            .get("nodes")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let conf = h
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("INFERRED");
        let cscore = h.get("confidence_score").and_then(Value::as_f64);
        let conf_tag = if let Some(cs) = cscore {
            format!("{conf} {cs:.2}")
        } else {
            conf.to_string()
        };
        let label = h
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| h.get("id").and_then(Value::as_str))
            .unwrap_or_default();
        lines.push(format!("- **{label}** — {node_labels} [{conf_tag}]"));
    }
}
