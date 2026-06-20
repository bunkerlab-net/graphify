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

/// File-level node ID matching the skill.md spec: `{parent_dir}_{stem}` — one
/// parent directory level, no extension.
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
