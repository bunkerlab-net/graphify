//! Unit tests for [`crate::graph_size`].
//!
//! Extracted from the inline `#[cfg(test)] mod tests { ... }` block
//! that used to live at the bottom of `graph_size.rs`. Behaviour is
//! unchanged; this layout matches the workspace convention that
//! tests live in dedicated `_tests.rs` (or `tests/parity.rs`) files.
//!
//! NOTE (`CodeRabbit` dispute): these cases exercise the *private* helpers
//! `parse_graph_byte_cap` and `format_with_underscores`, which are not part of
//! the crate's public API. An integration test under `tests/` cannot reach
//! them, and exposing them as `pub` purely to relocate the test would leak
//! internals. So this stays an in-crate unit-test module (compiled only under
//! `#[cfg(test)]` and pulled in via `#[path]`), which is itself a dedicated
//! test file — not inline with the production code it covers.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

use super::*;

#[test]
fn format_with_underscores_matches_python() {
    assert_eq!(format_with_underscores(0), "0");
    assert_eq!(format_with_underscores(16), "16");
    assert_eq!(format_with_underscores(999), "999");
    assert_eq!(format_with_underscores(1_000), "1_000");
    assert_eq!(format_with_underscores(536_870_912), "536_870_912");
}

#[test]
fn parse_graph_byte_cap_handles_suffixes_and_fallbacks() {
    // Blank / whitespace → default cap.
    assert_eq!(parse_graph_byte_cap(""), MAX_GRAPH_FILE_BYTES);
    assert_eq!(parse_graph_byte_cap("   "), MAX_GRAPH_FILE_BYTES);
    // Plain bytes.
    assert_eq!(parse_graph_byte_cap("671088640"), 671_088_640);
    // MB / GB suffixes, case-insensitive, 1024-based, optional spaces.
    assert_eq!(parse_graph_byte_cap("640MB"), 640 * 1024 * 1024);
    assert_eq!(parse_graph_byte_cap("2gb"), 2 * 1024 * 1024 * 1024);
    assert_eq!(parse_graph_byte_cap("2 GB"), 2 * 1024 * 1024 * 1024);
    // Zero / negative / garbage → default cap.
    assert_eq!(parse_graph_byte_cap("0"), MAX_GRAPH_FILE_BYTES);
    assert_eq!(parse_graph_byte_cap("-5"), MAX_GRAPH_FILE_BYTES);
    assert_eq!(parse_graph_byte_cap("not-a-number"), MAX_GRAPH_FILE_BYTES);
    assert_eq!(parse_graph_byte_cap("GB"), MAX_GRAPH_FILE_BYTES);
}
