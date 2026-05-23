//! Canonical `file_type` values and the synonym table used to coerce raw
//! extraction values into them.

use serde_json::Value;

/// Allowed canonical `file_type` values.
const VALID_FILE_TYPES: &[&str] = &["code", "document", "paper", "image", "rationale", "concept"];

/// Map a known invalid `file_type` value emitted by an LLM subagent to
/// its canonical equivalent.
///
/// Returns `None` for values that have no known synonym; callers fall
/// back to `"concept"` in that case.
fn file_type_synonym(s: &str) -> Option<&'static str> {
    match s {
        "markdown" | "text" => Some("document"),
        "tool" | "library" => Some("code"),
        "pattern" | "principle" | "constraint" | "tech" | "technology" | "data-source"
        | "data_source" | "gotcha" | "framework" => Some("concept"),
        _ => None,
    }
}

/// Coerce a raw `file_type` attribute value to a valid canonical string.
///
/// Returns `Some(replacement)` if the value needs replacing, or `None`
/// when the existing value is already canonical and the caller should
/// leave it untouched.
pub(crate) fn coerce_file_type(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => {
            if s.is_empty() {
                Some("concept".to_string())
            } else if VALID_FILE_TYPES.contains(&s.as_str()) {
                None
            } else {
                Some(file_type_synonym(s).unwrap_or("concept").to_string())
            }
        }
        _ => Some("concept".to_string()),
    }
}
