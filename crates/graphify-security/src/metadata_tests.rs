//! Unit tests for [`crate::metadata`].
//!
//! Extracted from the inline `#[cfg(test)] mod tests { ... }` block
//! that used to live at the bottom of `metadata.rs`. Behaviour is
//! unchanged; this layout matches the workspace convention that
//! tests live in dedicated `_tests.rs` (or `tests/parity.rs`) files.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

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
