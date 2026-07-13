//! Shrink-guard: refuses to overwrite graph output when the new node
//! count is strictly less than the previous one (matches Python's
//! `_check_shrink`, which also rejects any decrease, not just `>= 50%`).
//!
//! Extracted from `rebuild.rs` so the shrink-detection logic is isolated and
//! independently testable.

use std::collections::HashSet;
use std::path::Path;

use graphify_build::norm_source_file;
use serde_json::Value;

use crate::error::WatchError;

/// A shrink-guard function: the production pipeline uses [`check_shrink`];
/// `test_support` supplies a scoped rejecting variant so the marker-refusal
/// contract can be exercised without global state or environment coupling.
pub type ShrinkChecker = fn(
    bool,
    &Value,
    &Value,
    Option<&Path>,
    bool,
    Option<&HashSet<String>>,
) -> Result<(), WatchError>;
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
/// - `rebuilt_sources` is `Some` and every *lost* node (present before, gone
///   now) belonged to a source re-extracted this run or carries no
///   `source_file` — a symbol removed from a rebuilt file is a legitimate
///   shrink, not a failed chunk (#1116).
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
// `None` call sites cannot infer a generic hasher parameter, and every caller
// builds the set with the default hasher.
#[allow(clippy::implicit_hasher)]
pub fn check_shrink(
    force: bool,
    existing_data: &Value,
    new_data: &Value,
    tmp: Option<&Path>,
    had_explicit_deletions: bool,
    rebuilt_sources: Option<&HashSet<String>>,
) -> Result<(), WatchError> {
    if force || had_explicit_deletions {
        return Ok(());
    }
    let existing_nodes = existing_data.get("nodes").and_then(Value::as_array);
    let existing_count = existing_nodes.map_or(0, Vec::len);
    if existing_count == 0 {
        return Ok(());
    }
    let new_node_arr = new_data.get("nodes").and_then(Value::as_array);
    let new_count = new_node_arr.map_or(0, Vec::len);
    if new_count >= existing_count {
        return Ok(());
    }

    // A net shrink is legitimate — not a failed chunk — when every *lost* node
    // belonged to a source re-extracted this run (a symbol removed from a
    // rebuilt file) or carries no source_file. Only an unexplained loss (a node
    // from a file we did NOT touch) refuses the write. (#1116)
    if let Some(rebuilt) = rebuilt_sources
        && let Some(existing_arr) = existing_nodes
    {
        let new_ids: HashSet<&str> = new_node_arr
            .into_iter()
            .flatten()
            .filter_map(|n| n.get("id").and_then(Value::as_str))
            .collect();
        let all_lost_accounted = existing_arr.iter().all(|n| {
            // A node still present is not "lost".
            if n.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| new_ids.contains(id))
            {
                return true;
            }
            match n.get("source_file").and_then(Value::as_str) {
                None | Some("") => true,
                Some(sf) => rebuilt.contains(sf) || rebuilt.contains(&norm_source_file(sf, None)),
            }
        });
        if all_lost_accounted {
            return Ok(());
        }
    }

    if let Some(tmp_path) = tmp {
        let _ = std::fs::remove_file(tmp_path);
    }
    eprintln!(
        "[graphify] WARNING: new graph has {new_count} nodes but existing \
         graph.json has {existing_count}. Refusing to overwrite — you may be \
         missing chunk files from a previous session. \
         Pass --force to override."
    );
    Err(WatchError::ShrinkRefused {
        existing: existing_count,
        new: new_count,
    })
}
