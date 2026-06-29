//! ID helpers: `make_id`, `file_stem`.
//!
//! These mirror Python's `_make_id` and `_file_stem` exactly.

use std::path::Path;

/// Build a stable node ID from one or more name parts.
///
/// Mirrors Python `graphify.ids.make_id(*parts)` (#811 unification): stray
/// `_`/`.` are trimmed from each non-empty part, the parts are joined with `_`,
/// and the joined string is normalised by [`graphify_build::normalize_id`] — the
/// single shared recipe (NFKC → non-word→`_` → collapse `_` → strip → casefold)
/// the graph builder also uses, so the AST id-maker and the builder can no
/// longer drift.
#[must_use]
pub fn make_id(parts: &[&str]) -> String {
    let combined = parts
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.trim_matches(|c| c == '_' || c == '.'))
        .collect::<Vec<_>>()
        .join("_");
    graphify_build::normalize_id(&combined)
}

/// Convenience wrapper for a single part.
#[must_use]
pub fn make_id1(part: &str) -> String {
    make_id(&[part])
}

/// Return the file's full repo-relative path with the extension dropped, as a
/// POSIX string (forward slashes). [`make_id`] later collapses the separators to
/// underscores, so same-named files in different directories get distinct IDs
/// instead of colliding into one last-writer-wins node (#1504):
///
/// - `docs/v1/api/README.md` → `docs/v1/api/README` → `docs_v1_api_readme`
/// - `docs/v2/api/README.md` → `docs/v2/api/README` → `docs_v2_api_readme`
///
/// Top-level files keep a bare stem (`setup.py` → `setup`). When passed an
/// absolute path the whole path is encoded; the `extract()` id-remap post-pass
/// (see [`crate::extractors::multi`]) re-derives the canonical repo-relative
/// form from `source_file`, so the on-disk location can't leak into persisted
/// IDs (#502). Mirrors Python `_file_stem(path)`.
#[must_use]
pub fn file_stem(path: &Path) -> String {
    path.with_extension("").to_string_lossy().replace('\\', "/")
}

/// File-level node ID: the full repo-relative path joined with `_`, extension
/// dropped (`src/auth/session.py` → `src_auth_session`).
///
/// `rel_path` MUST be relative to the project root so top-level files collapse
/// to a bare stem (`setup.py` → `setup`) instead of picking up the root
/// directory name. This must equal the ID semantic subagents generate, or AST
/// and semantic extraction split a file into two disconnected ghost nodes
/// (#1033). Mirrors Python `_file_node_id`.
#[must_use]
pub fn file_node_id(rel_path: &Path) -> String {
    make_id1(&file_stem(rel_path))
}

#[cfg(test)]
#[path = "ids_tests.rs"]
mod ids_tests;
