//! Graph-load memory-bomb cap.
//!
//! Rejects on-disk graph files larger than the cap before they are read into
//! memory and JSON-parsed. Mirrors `graphify-py/graphify/security.py`
//! `check_graph_file_size_cap` / `_MAX_GRAPH_FILE_BYTES`.

use std::path::Path;

use crate::error::SecurityError;

/// Default cap on the size of a graph file before parsing. Matches Python's
/// `_MAX_GRAPH_FILE_BYTES = 512 * 1024 * 1024` (512 MiB). The *effective* cap
/// is resolved at call time by [`max_graph_file_bytes`], which lets
/// `GRAPHIFY_MAX_GRAPH_BYTES` override it.
pub const MAX_GRAPH_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Resolve the graph-file size cap in bytes, honoring `GRAPHIFY_MAX_GRAPH_BYTES`.
/// The value may be plain bytes (`671088640`) or carry an `MB` / `GB` suffix
/// (`640MB`, `2GB` — case-insensitive, binary multipliers: `MB` = 1 MiB =
/// 1024×1024 bytes, `GB` = 1 GiB = 1024×1024×1024 bytes). Falls back to
/// [`MAX_GRAPH_FILE_BYTES`] when the var is unset, blank, or unparseable. Read
/// fresh on every call so the var can be set before any cap check applies.
#[must_use]
pub fn max_graph_file_bytes() -> u64 {
    parse_graph_byte_cap(&std::env::var("GRAPHIFY_MAX_GRAPH_BYTES").unwrap_or_default())
}

/// Parse a `GRAPHIFY_MAX_GRAPH_BYTES` value into a byte cap, falling back to
/// [`MAX_GRAPH_FILE_BYTES`] for blank/zero/negative/unparseable input. Split
/// out from [`max_graph_file_bytes`] so the parsing can be tested without
/// mutating the process environment.
#[must_use]
fn parse_graph_byte_cap(raw: &str) -> u64 {
    let raw = raw.trim();
    if raw.is_empty() {
        return MAX_GRAPH_FILE_BYTES;
    }
    let text = raw.to_uppercase();
    let (num, multiplier) = if let Some(stripped) = text.strip_suffix("GB") {
        (stripped.trim(), 1024 * 1024 * 1024)
    } else if let Some(stripped) = text.strip_suffix("MB") {
        (stripped.trim(), 1024 * 1024)
    } else {
        (text.as_str(), 1)
    };
    match num.parse::<u64>() {
        Ok(value) if value > 0 => value.saturating_mul(multiplier),
        _ => MAX_GRAPH_FILE_BYTES,
    }
}

/// Reject the file at `path` if its size exceeds the effective graph-file cap
/// ([`max_graph_file_bytes`], honoring `GRAPHIFY_MAX_GRAPH_BYTES`).
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
    check_graph_file_size_cap_with(path, max_graph_file_bytes())
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
#[must_use]
fn format_with_underscores(value: u64) -> String {
    let digits = value.to_string();
    // `digits` comes from `u64::to_string`, which only emits ASCII decimal
    // digits, so every 3-byte slice from `rchunks(3)` is guaranteed-valid
    // UTF-8. The `.expect` documents that invariant rather than masking a
    // bug with `unwrap_or("")` — if it ever panics, something far worse
    // has happened to `u64::to_string`.
    #[allow(clippy::expect_used)] // invariant documented above
    let chunks: Vec<&str> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| std::str::from_utf8(c).expect("digits from u64::to_string must be valid UTF-8"))
        .collect();
    chunks.join("_")
}

#[cfg(test)]
#[path = "graph_size_tests.rs"]
mod graph_size_tests;
