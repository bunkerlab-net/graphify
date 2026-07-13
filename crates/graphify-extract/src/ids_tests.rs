//! Unit tests for [`crate::ids`] — the deterministic node-ID builders.
//!
//! Parity with Python's `graphify.extract._make_id` is critical: every node ID
//! the LLM and dedup phases see flows through these helpers.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

use super::*;

/// Leading dots and underscores are stripped so `_auth` and `.auth` collapse
/// to the same canonical token.
#[test]
fn make_id_strips_dots_and_underscores() {
    assert_eq!(make_id1("_auth"), "auth");
    assert_eq!(make_id(&[".httpx._client"]), "httpx_client");
}

/// The function must be pure — repeated input yields the same output.
#[test]
fn make_id_consistent() {
    assert_eq!(make_id(&["foo", "Bar"]), make_id(&["foo", "Bar"]));
}

/// IDs never start or end with `_`; Python's normaliser strips both ends.
#[test]
fn make_id_no_leading_trailing_underscores() {
    let result = make_id1("__init__");
    assert!(!result.starts_with('_'));
    assert!(!result.ends_with('_'));
}

/// A file under a subdirectory keeps its full repo-relative path (extension
/// dropped); `make_id` collapses the separators to `_` later (#1504).
#[test]
fn file_stem_full_relative_path() {
    let p = std::path::PathBuf::from("auth/models.py");
    assert_eq!(file_stem(&p), "auth/models");
}

/// A root-level file gets a bare stem (no directory prefix).
#[test]
fn file_stem_root_level() {
    let p = std::path::PathBuf::from("models.py");
    assert_eq!(file_stem(&p), "models");
}
/// A path with no file name (`Path(".")` — a `source_file` equal to the scan
/// root) has no per-file stem, so `file_stem` returns "" instead of raising or
/// leaking `.` into a node id (#1618).
#[test]
fn file_stem_handles_dot_path() {
    use std::path::Path;
    assert_eq!(file_stem(Path::new(".")), "");
    assert_eq!(file_stem(Path::new("src/foo.py")), "src/foo");
}
