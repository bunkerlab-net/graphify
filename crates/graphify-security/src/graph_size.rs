//! Graph-load memory-bomb cap.
//!
//! Rejects on-disk graph files larger than the cap before they are read into
//! memory and JSON-parsed. Mirrors `graphify-py/graphify/security.py`
//! `check_graph_file_size_cap` / `_MAX_GRAPH_FILE_BYTES`.

use std::path::Path;

use crate::error::SecurityError;

/// Hard cap on the size of a graph file before parsing. Matches Python's
/// `_MAX_GRAPH_FILE_BYTES = 512 * 1024 * 1024` (512 MiB).
pub const MAX_GRAPH_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Reject the file at `path` if its size exceeds [`MAX_GRAPH_FILE_BYTES`].
///
/// Silently returns `Ok(())` when `path.metadata()` cannot be read — the
/// caller's own existence check is expected to surface a clearer error in
/// that case.
///
/// # Errors
///
/// Returns [`SecurityError::GraphFileTooLarge`] if the file size strictly
/// exceeds the cap. Equal-to-cap passes.
pub fn check_graph_file_size_cap(path: &Path) -> Result<(), SecurityError> {
    check_graph_file_size_cap_with(path, MAX_GRAPH_FILE_BYTES)
}

/// Variant of [`check_graph_file_size_cap`] that takes an explicit cap.
///
/// Mirrors Python's monkeypatching pattern in `test_security.py` where the
/// `_MAX_GRAPH_FILE_BYTES` constant is temporarily overridden.
///
/// # Errors
///
/// Returns [`SecurityError::GraphFileTooLarge`] if the file size strictly
/// exceeds `cap`. Equal-to-cap passes.
pub fn check_graph_file_size_cap_with(path: &Path, cap: u64) -> Result<(), SecurityError> {
    let Ok(meta) = path.metadata() else {
        return Ok(());
    };
    let size = meta.len();
    if size > cap {
        return Err(SecurityError::GraphFileTooLarge {
            path: path.to_path_buf(),
            size: format_with_underscores(size),
            cap: format_with_underscores(cap),
        });
    }
    Ok(())
}

/// Format a `u64` using underscore thousand separators, matching Python's
/// `f"{value:_d}"`. Implemented as a right-aligned chunked walk
/// (`.rchunks(3).rev()`) so the separator placement is obvious at a
/// glance.
fn format_with_underscores(value: u64) -> String {
    let digits = value.to_string();
    // `digits` comes from `u64::to_string`, which only emits ASCII decimal
    // digits, so every 3-byte slice from `rchunks(3)` is guaranteed-valid
    // UTF-8. The `unwrap_or("")` fallback in the map below is therefore
    // unreachable in practice — kept only so this helper never panics on
    // future refactors that change `digits`'s source.
    let chunks: Vec<&str> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();
    chunks.join("_")
}

#[cfg(test)]
#[path = "graph_size_tests.rs"]
mod graph_size_tests;
