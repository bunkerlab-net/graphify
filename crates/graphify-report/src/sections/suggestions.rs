//! Suggested-questions section renderer.
//!
//! Extracted from `lib.rs` to isolate the LLM-generated question block.

use serde_json::Value;

/// Render the "Suggested Questions" section.
pub(crate) fn render_questions(lines: &mut Vec<String>, suggested_questions: &[Value]) {
    lines.push(String::new());
    lines.push("## Suggested Questions".to_string());
    let no_signal = suggested_questions.len() == 1
        && suggested_questions
            .first()
            .and_then(|q| q.get("type"))
            .and_then(Value::as_str)
            == Some("no_signal");
    if no_signal {
        let why = suggested_questions
            .first()
            .and_then(|q| q.get("why"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        lines.push(format!("_{why}_"));
    } else {
        lines.push("_Questions this graph is uniquely positioned to answer:_".to_string());
        lines.push(String::new());
        for q in suggested_questions {
            if let Some(question) = q.get("question").and_then(Value::as_str)
                && !question.is_empty()
            {
                lines.push(format!("- **{question}**"));
                let why = q.get("why").and_then(Value::as_str).unwrap_or_default();
                lines.push(format!("  _{why}_"));
            }
        }
    }
}
