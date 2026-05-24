//! Unit tests for [`crate::graph_size`].
//!
//! Extracted from the inline `#[cfg(test)] mod tests { ... }` block
//! that used to live at the bottom of `graph_size.rs`. Behaviour is
//! unchanged; this layout matches the workspace convention that
//! tests live in dedicated `_tests.rs` (or `tests/parity.rs`) files.

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
