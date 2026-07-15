//! Explicit, per-call test-injection seam.
//!
//! graphify-py's `_rebuild_code` marker test monkeypatches `_check_shrink` to
//! force a write refusal. Rust has no runtime patching, and a genuine
//! *unexplained* shrink is unreachable through the reconcile path (every lost
//! node is excused as a rebuilt/deleted source), so this module runs the real
//! rebuild pipeline with a rejecting shrink-guard injected for exactly one call.
//!
//! Nothing here is reachable from `rebuild_code`, and no global state or
//! environment variable is involved — a production binary always uses the real
//! [`crate::rebuild::check_shrink`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::WatchError;
use crate::rebuild::{RebuildOptions, rebuild_code_impl};

/// A [`crate::rebuild::ShrinkChecker`] that always refuses, cleaning up the
/// candidate temp file like the real guard does.
fn always_refuse(
    _force: bool,
    existing: &Value,
    _new: &Value,
    tmp: Option<&Path>,
    _had_explicit_deletions: bool,
    _rebuilt_sources: Option<&HashSet<String>>,
) -> Result<(), WatchError> {
    if let Some(tmp_path) = tmp {
        let _ = std::fs::remove_file(tmp_path);
    }
    let existing = existing
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Err(WatchError::ShrinkRefused { existing, new: 0 })
}

/// Run the full rebuild pipeline with the shrink guard forced to refuse, so the
/// "`.graphify_root` marker is not updated when a write is refused" contract can
/// be exercised deterministically. The refusal is scoped to this single call.
///
/// # Errors
///
/// Propagates rebuild errors, including the forced
/// [`WatchError::ShrinkRefused`].
pub fn rebuild_code_forcing_shrink_refusal(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    opts: RebuildOptions,
) -> Result<bool, WatchError> {
    rebuild_code_impl(watch_path, changed_paths, opts, always_refuse)
}
