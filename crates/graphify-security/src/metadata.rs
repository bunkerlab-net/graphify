//! Recursive metadata sanitisation: control-char strip, HTML-escape,
//! length / list caps, dropped-empty-key handling.
//!
//! Mirrors `graphify-py/graphify/security.py` `sanitize_metadata` /
//! `_sanitize_metadata_value` / `_sanitize_metadata_string`.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

/// Maximum length (in chars / code points) of a sanitised metadata string.
pub const METADATA_MAX_VALUE_LEN: usize = 512;

/// Maximum number of items kept from any list or tuple value.
pub const METADATA_MAX_LIST_ITEMS: usize = 50;

#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static CONTROL_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x00-\x1f\x7f]").expect("literal pattern is valid"));

/// Sanitise a single metadata string: strip control chars, HTML-escape with
/// quote handling, cap at [`METADATA_MAX_VALUE_LEN`] code points.
///
/// Matches Python's `_sanitize_metadata_string`, including the order of
/// operations (escape after strip, cap last).
#[must_use]
pub fn sanitize_metadata_string(value: &str) -> String {
    let stripped = CONTROL_CHARS.replace_all(value, "");
    let escaped = html_escape_with_quotes(&stripped);
    cap_chars(&escaped, METADATA_MAX_VALUE_LEN)
}

/// Sanitise a metadata value, preserving JSON-compatible scalar types.
///
/// Recurses into dicts (objects). Lists/tuples are capped at
/// [`METADATA_MAX_LIST_ITEMS`] and each element is sanitised recursively.
/// `bool`, integer, float, and `null` pass through unchanged.
#[must_use]
pub fn sanitize_metadata_value(value: &Value) -> Value {
    match value {
        Value::Bool(b) => Value::Bool(*b),
        Value::Number(n) => Value::Number(n.clone()),
        Value::Null => Value::Null,
        Value::String(s) => Value::String(sanitize_metadata_string(s)),
        Value::Object(map) => Value::Object(sanitize_metadata_map(map)),
        Value::Array(items) => {
            let limited: Vec<Value> = items
                .iter()
                .take(METADATA_MAX_LIST_ITEMS)
                .map(sanitize_metadata_value)
                .collect();
            Value::Array(limited)
        }
    }
}

/// Sanitise a metadata mapping by sanitising every key and every value.
/// Entries whose key becomes empty after sanitisation are dropped.
#[must_use]
pub fn sanitize_metadata_map(map: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::with_capacity(map.len());
    for (key, value) in map {
        let clean_key = sanitize_metadata_string(key);
        if clean_key.is_empty() {
            continue;
        }
        out.insert(clean_key, sanitize_metadata_value(value));
    }
    out
}

/// Sanitise a metadata mapping, accepting `None` (returns an empty map).
///
/// Convenience entry point for callers that hold an `Option<&Map<…>>`.
#[must_use]
pub fn sanitize_metadata(metadata: Option<&Map<String, Value>>) -> Map<String, Value> {
    match metadata {
        Some(map) => sanitize_metadata_map(map),
        None => Map::new(),
    }
}

/// Escape `&`, `<`, `>`, `"`, and `'` in the manner of Python's
/// `html.escape(text, quote=True)`.
///
/// `&` must be replaced first to avoid double-escaping the subsequent
/// entities. Apostrophe maps to `&#x27;` — matching `CPython`'s choice and
/// the parity-test substring `&#x27;`.
fn html_escape_with_quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            other => out.push(other),
        }
    }
    out
}

/// Truncate `text` to at most `max_chars` code points (Python `len(str)`
/// semantics, not graphemes). Always returns the built `String` (whether
/// or not truncation actually occurred) so the function never needs to
/// allocate a second time.
fn cap_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_escapes_html_specials() {
        let result = sanitize_metadata_string("<script>alert('x')</script>");
        assert!(result.contains("&lt;"));
        assert!(result.contains("&gt;"));
        assert!(result.contains("&#x27;"));
        assert!(!result.contains("<script>"));
    }

    #[test]
    fn string_strips_control_chars() {
        let result = sanitize_metadata_string("hello\x00\x1fworld");
        assert!(!result.contains('\u{0}'));
        assert!(!result.contains('\u{1f}'));
        assert!(result.contains("helloworld"));
    }

    #[test]
    fn string_caps_at_512_chars() {
        let long: String = std::iter::repeat_n('a', METADATA_MAX_VALUE_LEN + 100).collect();
        let result = sanitize_metadata_string(&long);
        assert!(result.chars().count() <= METADATA_MAX_VALUE_LEN);
    }

    #[test]
    fn value_preserves_simple_scalars() {
        assert_eq!(sanitize_metadata_value(&json!(42)), json!(42));
        assert_eq!(sanitize_metadata_value(&json!(2.5)), json!(2.5));
        assert_eq!(sanitize_metadata_value(&json!(true)), json!(true));
        assert_eq!(sanitize_metadata_value(&json!(false)), json!(false));
        assert_eq!(sanitize_metadata_value(&Value::Null), Value::Null);
    }

    #[test]
    fn value_caps_list_length() {
        let items: Vec<Value> = (0..METADATA_MAX_LIST_ITEMS * 3).map(|n| json!(n)).collect();
        let out = sanitize_metadata_value(&Value::Array(items));
        let Value::Array(out_items) = out else {
            panic!("expected array");
        };
        assert_eq!(out_items.len(), METADATA_MAX_LIST_ITEMS);
    }

    #[test]
    fn map_drops_empty_keys() {
        let mut map = Map::new();
        map.insert("\x00".to_owned(), json!("v"));
        map.insert("k".to_owned(), json!("v2"));
        let out = sanitize_metadata_map(&map);
        assert!(!out.contains_key("\x00"));
        assert_eq!(out.get("k"), Some(&json!("v2")));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn map_sanitises_keys() {
        let mut map = Map::new();
        map.insert("<bad>".to_owned(), json!("v"));
        let out = sanitize_metadata_map(&map);
        assert!(!out.contains_key("<bad>"));
        assert!(out.keys().any(|k| k.contains("&lt;")));
    }

    #[test]
    fn none_returns_empty_map() {
        assert!(sanitize_metadata(None).is_empty());
    }
}
