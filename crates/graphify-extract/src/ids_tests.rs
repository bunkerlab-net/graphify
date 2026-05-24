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

/// A file under a subdirectory is qualified by its parent directory name so
/// `auth/models.py` and `models.py` produce distinct stems.
#[test]
fn file_stem_qualifies_with_parent() {
    let p = std::path::PathBuf::from("/project/auth/models.py");
    assert_eq!(file_stem(&p), "auth.models");
}

/// A root-level file gets no parent prefix.
#[test]
fn file_stem_root_level() {
    let p = std::path::PathBuf::from("models.py");
    assert_eq!(file_stem(&p), "models");
}
