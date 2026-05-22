//! Rebuild pipeline entry point and public re-exports.
//!
//! This module is the `mod rebuild;` target from `lib.rs`.  It owns the
//! public `rebuild_code` function (lock acquisition + dispatch) and
//! re-exports the helper functions used by tests and downstream crates.
//! The heavy pipeline logic lives in `pipeline.rs`.

pub mod community;
pub mod git;
pub mod helpers;
pub mod pipeline;
pub mod relativize;
pub mod shrink;

pub use community::node_community_map;
pub use git::git_head;
pub use relativize::relativize_source_files;
pub use shrink::check_shrink;

use std::path::{Path, PathBuf};

use crate::error::WatchError;
use crate::graphify_out;
use crate::lock::RebuildLock;

use pipeline::rebuild_code_inner;

/// Re-run AST extraction + build + optional cluster + report for code files.
///
/// Acquires a per-repo advisory lock (unless `acquire_lock` is `false`).
/// Returns `Ok(true)` when outputs were updated, `Ok(false)` when the rebuild
/// was skipped (lock held, no tracked files changed, shrink guard refused).
///
/// See module-level doc for the full pipeline description.
///
/// # Errors
///
/// Propagates I/O and pipeline errors via [`WatchError`].
#[allow(clippy::fn_params_excessive_bools)]
// reason: mirrors Python's _rebuild_code signature 1:1; each bool controls a
// distinct pipeline flag; extracting enums would diverge from the reference spec.
pub fn rebuild_code(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    _follow_symlinks: bool,
    force: bool,
    no_cluster: bool,
    acquire_lock: bool,
    block_on_lock: bool,
) -> Result<bool, WatchError> {
    let out = watch_path.join(graphify_out());

    if acquire_lock {
        let guard = RebuildLock::acquire(&out, block_on_lock)?;
        if !guard.acquired() {
            println!(
                "[graphify watch] Rebuild already in progress for {} - skipping.",
                watch_path
                    .canonicalize()
                    .unwrap_or_else(|_| watch_path.to_path_buf())
                    .display()
            );
            return Ok(false);
        }
        let result = rebuild_code_inner(watch_path, changed_paths, force, no_cluster);
        drop(guard);
        result
    } else {
        rebuild_code_inner(watch_path, changed_paths, force, no_cluster)
    }
}
