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

    // Strategy 2: scan every balanced `{ … }` object in the original trimmed
    // text and pick the best parseable one. Handles JSON wrapped in prose with
    // no fence, e.g. "The extracted graph is { ... }. Hope this helps!", and —
    // unlike a first-object-only scan — skips an incidental brace group that
    // appears before the real payload (e.g. "Note {see below}. Graph: {...}"),
    // which would otherwise fail to parse and abort the whole strategy. Using
    // the original (not the fence-stripped) text keeps a ```-in-string payload
    // intact. An extraction-shaped object (one carrying `nodes`/`edges`/
    // `hyperedges`) is preferred over any other valid object so an incidental
    // `{"status": "ok"}` does not shadow the real fragment.
    let mut first_parsed: Option<Value> = None;
    for obj in balanced_objects(trimmed) {
        let Ok(v) = serde_json::from_str::<Value>(obj) else {
            continue;
        };
        if v.get("nodes").is_some() || v.get("edges").is_some() || v.get("hyperedges").is_some() {
            return v;
        }
        if first_parsed.is_none() {
            first_parsed = Some(v);
        }
    }
    if let Some(v) = first_parsed {
        return v;
    }

    let preview: String = raw.chars().take(200).collect();
    eprintln!(
        "[graphify] LLM returned invalid JSON, skipping chunk (first 200 chars: {preview:?})"
    );
    empty_fragment()
}

/// Return every top-level balanced `{ … }` substring of `s`, in source order.
///
/// Quotes and backslash escapes inside strings are respected so braces within
/// string literals do not affect depth. Objects nested inside another are part
/// of their enclosing candidate, not separate entries. An unbalanced trailing
/// `{` (e.g. a truncated response) yields no further candidates.
fn balanced_objects(s: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find('{') {
        let start = search_from + rel;
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escape = false;
        let mut end = None;
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
                        end = Some(start + i + ch.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(e) => {
                out.push(&s[start..e]);
                search_from = e;
            }
            // Unbalanced from here on; no later candidate can close either.
            None => break,
        }
    }
    out
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
