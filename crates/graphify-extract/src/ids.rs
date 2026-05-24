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
mod tests {
    use super::*;

    #[test]
    fn make_id_strips_dots_and_underscores() {
        assert_eq!(make_id1("_auth"), "auth");
        assert_eq!(make_id(&[".httpx._client"]), "httpx_client");
    }

    #[test]
    fn make_id_consistent() {
        assert_eq!(make_id(&["foo", "Bar"]), make_id(&["foo", "Bar"]));
    }

    #[test]
    fn make_id_no_leading_trailing_underscores() {
        let result = make_id1("__init__");
        assert!(!result.starts_with('_'));
        assert!(!result.ends_with('_'));
    }

    #[test]
    fn file_stem_qualifies_with_parent() {
        let p = std::path::PathBuf::from("/project/auth/models.py");
        assert_eq!(file_stem(&p), "auth.models");
    }

    #[test]
    fn file_stem_root_level() {
        let p = std::path::PathBuf::from("models.py");
        assert_eq!(file_stem(&p), "models");
    }
}
