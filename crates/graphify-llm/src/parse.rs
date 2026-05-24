//! JSON parsing helpers for LLM responses.
//!
//! Extracted from `lib.rs` to isolate `parse_llm_json` (fence-stripping and
//! size-capped JSON parse) and `response_is_hollow` (empty-result detection).

use serde_json::{Value, json};

use crate::LLM_JSON_MAX_BYTES;

/// Return an empty extraction fragment.
#[must_use]
pub fn empty_fragment() -> Value {
    json!({"nodes": [], "edges": [], "hyperedges": []})
}

/// Strip optional markdown fences and parse JSON.
///
/// Returns an empty fragment on failure. Capped at [`LLM_JSON_MAX_BYTES`].
#[must_use]
pub fn parse_llm_json(raw: &str) -> Value {
    if raw.len() > LLM_JSON_MAX_BYTES {
        eprintln!(
            "[graphify] LLM response exceeds {LLM_JSON_MAX_BYTES} bytes \
             ({} bytes); refusing to parse and dropping chunk.",
            raw.len()
        );
        return empty_fragment();
    }
    let mut s = raw.trim();
    if s.starts_with("```") {
        let parts: Vec<&str> = s.splitn(3, "```").collect();
        if parts.len() >= 2 {
            let mut inner = parts[1];
            if inner.starts_with("json") {
                inner = &inner[4..];
            }
            // Strip trailing fence
            let trimmed = inner.trim();
            if let Some(idx) = trimmed.rfind("```") {
                s = trimmed[..idx].trim();
            } else {
                s = trimmed;
            }
        }
    }
    match serde_json::from_str::<Value>(s) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[graphify] LLM returned invalid JSON, skipping chunk: {e}");
            empty_fragment()
        }
    }
}

/// Return `true` if the response produced no usable nodes, edges, or hyperedges.
#[must_use]
pub fn response_is_hollow(raw_content: Option<&str>, parsed: &Value) -> bool {
    match raw_content {
        None => return true,
        Some(s) if s.trim().is_empty() => return true,
        Some(_) => {}
    }
    let is_empty_arr = |key: &str| {
        parsed
            .get(key)
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    };
    is_empty_arr("nodes") && is_empty_arr("edges") && is_empty_arr("hyperedges")
}
