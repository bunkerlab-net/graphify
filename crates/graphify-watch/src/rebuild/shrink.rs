//! Shrink-guard: refuses to overwrite graph output when the new node
//! count is strictly less than the previous one (matches Python's
//! `_check_shrink`, which also rejects any decrease, not just `>= 50%`).
//!
//! Extracted from `rebuild.rs` so the shrink-detection logic is isolated and
//! independently testable.

use std::path::Path;

use serde_json::Value;

use crate::error::WatchError;

/// Return `Ok(())` when the node count is acceptable, `Err` when the new graph
/// has shrunk relative to the existing one.
///
/// The guard exists to catch SILENT shrinkage from failed extraction chunks
/// (a half-written semantic pass leaving thousands of nodes unaccounted for).
/// It is bypassed when:
///
/// - `force` is `true` (caller has explicitly opted out of the guard);
/// - there is no existing graph (empty `existing_data`);
/// - `had_explicit_deletions` is `true`, signalling that the caller already
///   declared which files were removed (e.g. the post-commit hook saw a `D`
///   in `git diff --name-only`) and the smaller graph is the expected outcome.
///
/// If `tmp` is provided and the check fails, the temporary file is cleaned up
/// before returning `Err`.
///
/// Ports `_check_shrink` from `watch.py:243-263`, but returns `Result` instead
/// of a boolean to integrate naturally with `?` propagation.
///
/// # Errors
///
/// Returns [`WatchError::ShrinkRefused`] when the new graph has fewer nodes
/// than the existing one and none of the bypass conditions apply.
pub fn check_shrink(
    force: bool,
    existing_data: &Value,
    new_data: &Value,
    tmp: Option<&Path>,
    had_explicit_deletions: bool,
) -> Result<(), WatchError> {
    if force || had_explicit_deletions {
        return Ok(());
    }
    let existing_nodes = existing_data
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if existing_nodes == 0 {
        return Ok(());
    }
    let new_nodes = new_data
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if new_nodes < existing_nodes {
        if let Some(tmp_path) = tmp {
            let _ = std::fs::remove_file(tmp_path);
        }
        eprintln!(
            "[graphify] WARNING: new graph has {new_nodes} nodes but existing \
             graph.json has {existing_nodes}. Refusing to overwrite — you may be \
             missing chunk files from a previous session. \
             Pass --force to override."
        );
        return Err(WatchError::ShrinkRefused {
            existing: existing_nodes,
            new: new_nodes,
        });
    }
    Ok(())
}
