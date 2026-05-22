//! ID and `source_file` normalisation shared with `graphify-extract`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

// `[^\w]+` with re.UNICODE means "any non-word run". The regex crate's default
// is Unicode-aware unless the `unicode` feature is disabled, so `\w` matches
// letters, digits, and `_` across all scripts — same as Python.
#[allow(clippy::expect_used)] // literal regex pattern; build cannot panic.
static NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w]+").expect("static non-word regex"));
#[allow(clippy::expect_used)] // literal regex pattern; build cannot panic.
static UNDERSCORE_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_+").expect("static underscore-run regex"));

/// Normalize an ID string the same way `extract._make_id` does. NFKC →
/// non-word→`_` → collapse repeated `_` → strip→casefold.
///
/// Must stay in sync with `extract._make_id` and `build._normalize_id` in
/// the Python reference.
#[must_use]
pub fn normalize_id(s: &str) -> String {
    let nfkc: String = s.nfkc().collect();
    let cleaned = NON_WORD.replace_all(&nfkc, "_");
    let collapsed = UNDERSCORE_RUN.replace_all(&cleaned, "_");
    collapsed.trim_matches('_').to_lowercase()
}

/// Normalize a `source_file` path: backslashes → forward slashes; absolute
/// paths inside `root` become root-relative.
#[must_use]
pub fn norm_source_file(p: &str, root: Option<&str>) -> String {
    if p.is_empty() {
        return p.to_string();
    }
    let mut out = p.replace('\\', "/");
    if let Some(r) = root {
        let path = Path::new(&out);
        if path.is_absolute() {
            let root_path = PathBuf::from(r);
            if let Ok(rel) = path.strip_prefix(&root_path) {
                out = rel.to_string_lossy().replace('\\', "/");
            }
        }
    }
    out
}
