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
mod pipeline_helpers;
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

/// Advisory-lock policy for [`rebuild_code`].
#[derive(Debug, Clone, Copy, Default)]
pub enum LockPolicy {
    /// Do not acquire the per-repo lock at all.
    None,
    /// Acquire the lock if free; skip the rebuild otherwise.
    #[default]
    TryAcquire,
    /// Acquire the lock, blocking until it becomes available.
    BlockOn,
}

/// Flag bundle for [`rebuild_code`].
///
/// Mirrors the relevant parameters from Python's `_rebuild_code`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RebuildOptions {
    /// Bypass the shrink-guard so a rebuild with fewer nodes is allowed.
    pub force: bool,
    /// Skip the community-detection step.
    pub no_cluster: bool,
    /// Lock-acquisition policy.
    pub lock: LockPolicy,
}

/// Re-run AST extraction + build + optional cluster + report for code files.
///
/// Acquires a per-repo advisory lock unless `opts.lock` is [`LockPolicy::None`].
/// Returns `Ok(true)` when outputs were updated, `Ok(false)` when the rebuild
/// was skipped (lock held, no tracked files changed, shrink guard refused).
///
/// See module-level doc for the full pipeline description.
///
/// # Errors
///
/// Propagates I/O and pipeline errors via [`WatchError`].
pub fn rebuild_code(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    opts: RebuildOptions,
) -> Result<bool, WatchError> {
    let out = watch_path.join(graphify_out());

    match opts.lock {
        LockPolicy::None => {
            rebuild_code_inner(watch_path, changed_paths, opts.force, opts.no_cluster)
        }
        LockPolicy::TryAcquire | LockPolicy::BlockOn => {
            let block = matches!(opts.lock, LockPolicy::BlockOn);
            let guard = RebuildLock::acquire(&out, block)?;
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
            let result = rebuild_code_inner(watch_path, changed_paths, opts.force, opts.no_cluster);
            drop(guard);
            result
        }
    }
}
