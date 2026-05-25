//! ID helpers: `make_id`, `file_stem`.
//!
//! These mirror Python's `_make_id` and `_file_stem` exactly.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w]+").expect("static non-word regex"));
#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static UNDERSCORE_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_+").expect("static underscore-run regex"));

/// Build a stable node ID from one or more name parts.
///
/// Mirrors Python `_make_id(*parts)`:
/// - joins non-empty parts with `_`
/// - NFKC-normalises
/// - collapses non-word runs to `_`
/// - strips leading/trailing `_`
/// - casefolds
#[must_use]
pub fn make_id(parts: &[&str]) -> String {
    let combined: String = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_matches(|c| c == '_' || c == '.'))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let nfkc: String = combined.nfkc().collect();
    let cleaned = NON_WORD.replace_all(&nfkc, "_");
    let collapsed = UNDERSCORE_RUN.replace_all(&cleaned, "_");
    collapsed.trim_matches('_').to_lowercase()
}

/// Convenience wrapper for a single part.
#[must_use]
pub fn make_id1(part: &str) -> String {
    make_id(&[part])
}

/// Return a stem qualified with the parent directory name to avoid ID
/// collisions when multiple files share the same filename in different
/// directories. Mirrors Python `_file_stem(path)`.
#[must_use]
pub fn file_stem(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !parent_name.is_empty() && parent_name != "." {
        format!("{parent_name}.{stem}")
    } else {
        stem.into_owned()
    }
}

#[cfg(test)]
#[path = "ids_tests.rs"]
mod ids_tests;
