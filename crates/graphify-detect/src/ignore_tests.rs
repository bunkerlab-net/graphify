//! Unit tests for [`crate::ignore`].
//!
//! Covers the small parsing/glob helpers; the larger end-to-end ignore-vs-include
//! scenarios are exercised by `tests/parity_ignore.rs` and so are intentionally
//! omitted here.

#![allow(clippy::expect_used)] // test-only — `.expect("...")` panics are the failure

use super::*;

/// Lines that start with `#` are treated as comments (returns empty string).
#[test]
fn parse_comment_line() {
    assert_eq!(parse_gitignore_line("# this is a comment"), "");
}

/// Empty input parses to empty output without error.
#[test]
fn parse_blank_line() {
    assert_eq!(parse_gitignore_line(""), "");
}

/// Trailing-slash directory patterns pass through untouched.
#[test]
fn parse_normal_pattern() {
    assert_eq!(parse_gitignore_line("vendor/"), "vendor/");
}

/// Backslash-escaped hashes (`\#`) must survive parsing so the pattern still
/// matches files with literal `#` in their names.
#[test]
fn parse_escaped_hash() {
    assert_eq!(parse_gitignore_line("path\\#hash.py"), "path#hash.py");
}

/// `*` matches within a single path component but not across `/`.
#[test]
fn glob_match_star() {
    assert!(glob_match("foo.py", "*.py"));
    assert!(!glob_match("foo/bar.py", "*.py"));
}

/// `**` is the cross-segment wildcard for recursive matching.
#[test]
fn glob_match_double_star() {
    assert!(glob_match("a/b/c.py", "**/*.py"));
}
