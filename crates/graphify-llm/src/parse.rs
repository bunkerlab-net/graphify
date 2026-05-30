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

/// Parse a JSON extraction fragment out of a raw LLM response.
///
/// Robust against the common failure modes Claude exhibits: a markdown fence
/// preceded by a prose preamble, prose-wrapped JSON with no fence at all, and
/// truncated responses missing their closing fence. Already-valid JSON is
/// parsed verbatim before any fence stripping, so a payload that legitimately
/// contains a triple-backtick substring inside a string is not corrupted.
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

    let trimmed = raw.trim();

    // Strategy 0: already-valid JSON. Try this *before* any fence stripping so a
    // response that genuinely is valid JSON — but happens to contain a ```
    // substring inside a string value — is parsed verbatim rather than mangled
    // by the fence logic below.
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return v;
    }

    // Strategy 1: handle a markdown fence found anywhere in the text (not only
    // at offset 0). Claude often prepends a preamble such as "Here are the
    // extracted entities:\n\n```json\n{...}\n```".
    if let Some(fence_start) = trimmed.find("```") {
        let mut after_fence = &trimmed[fence_start + 3..];
        // Optional language tag (json, JSON, javascript, js, …) up to newline.
        if let Some(nl) = after_fence.find('\n') {
            let tag = after_fence[..nl].trim().to_ascii_lowercase();
            if matches!(tag.as_str(), "json" | "javascript" | "js" | "") {
                after_fence = &after_fence[nl + 1..];
            }
        }
        let stripped = match after_fence.rfind("```") {
            Some(fence_end) => after_fence[..fence_end].trim(),
            None => after_fence.trim(),
        };
        if let Ok(v) = serde_json::from_str::<Value>(stripped) {
            return v;
        }
    }

    // Strategy 2: extract the first balanced JSON object found anywhere in the
    // original trimmed text. Handles JSON wrapped in prose with no fence, e.g.
    // "The extracted graph is { ... }. Hope this helps!". Using the original
    // (not the fence-stripped) text keeps a ```-in-string payload intact.
    if let Some(obj) = first_balanced_object(trimmed)
        && let Ok(v) = serde_json::from_str::<Value>(obj)
    {
        return v;
    }

    let preview: String = raw.chars().take(200).collect();
    eprintln!(
        "[graphify] LLM returned invalid JSON, skipping chunk (first 200 chars: {preview:?})"
    );
    empty_fragment()
}

/// Return the first balanced `{ … }` substring of `s`, or `None` when no
/// brace-balanced object can be found. Quotes and backslash escapes inside
/// strings are respected so braces within string literals do not affect depth.
fn first_balanced_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in s[start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' => in_string = !in_string,
            _ if in_string => {}
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let end = start + i + ch.len_utf8();
                    return Some(&s[start..end]);
                }
            }
            _ => {}
        }
    }
    None
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
