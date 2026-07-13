//! Rebuild pipeline entry point and public re-exports.
//!
//! This module is the `mod rebuild;` target from `lib.rs`.  It owns the
//! public `rebuild_code` function (lock acquisition + dispatch) and
//! re-exports the helper functions used by tests and downstream crates.
//! The heavy pipeline logic lives in `pipeline.rs`.

pub mod community;
pub mod git;
pub mod helpers;
pub mod pending;
pub mod pipeline;
mod pipeline_helpers;
pub mod reconcile;
pub mod relativize;
pub mod shrink;

pub use community::node_community_map;
pub use git::git_head;
pub use pending::{
    PENDING_DRAIN_MAX_PASSES, PENDING_FILENAME, drain_pending, merge_changed_paths, queue_pending,
    rebuild_with_pending,
};
pub use relativize::relativize_source_files;
pub use shrink::check_shrink;

use std::path::{Path, PathBuf};

use crate::error::WatchError;
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
    /// Follow symlinked directories during detection (mirrors graphify-py
    /// `_rebuild_code(follow_symlinks=...)`).
    pub follow_symlinks: bool,
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
    if !stabilize_rebuild_cwd(watch_path) {
        return Ok(false);
    }
    let out = watch_path.join(graphify_security::graphify_out());

    match opts.lock {
        LockPolicy::None => rebuild_code_inner(
            watch_path,
            changed_paths,
            opts.force,
            opts.no_cluster,
            opts.follow_symlinks,
        ),
        LockPolicy::TryAcquire | LockPolicy::BlockOn => {
            let block = matches!(opts.lock, LockPolicy::BlockOn);
            // #1059: an incremental hook must not drop its change set when
            // another rebuild is already running. Queue before attempting a
            // non-blocking lock so a failed acquisition still records the work;
            // the lock-holder drains the queue and merges it in. Full-corpus
            // rebuilds (changed_paths == None) skip the queue — they already
            // cover every file, so there is nothing to merge.
            if !block && let Some(paths) = changed_paths {
                pending::queue_pending(&out, paths)?;
            }
            let guard = RebuildLock::acquire(&out, block)?;
            if !guard.acquired() {
                println!(
                    "[graphify watch] Rebuild already in progress for {} - changes queued.",
                    watch_path
                        .canonicalize()
                        .unwrap_or_else(|_| watch_path.to_path_buf())
                        .display()
                );
                return Ok(false);
            }
            // Lock acquired. Drain anything queued by earlier contenders
            // (including the paths we just queued ourselves) and merge with our
            // own change set, then loop to absorb any late arrivals.
            let result = pending::rebuild_with_pending(&out, changed_paths, |paths| {
                rebuild_code_inner(
                    watch_path,
                    paths,
                    opts.force,
                    opts.no_cluster,
                    opts.follow_symlinks,
                )
            });
            drop(guard);
            result
        }
    }
}

/// Ensure relative rebuild paths have a usable CWD before queue/lock setup.
///
/// Detached git hooks can inherit a transient working directory that is deleted
/// before the background rebuild starts; in that state `current_dir()` and the
/// relative `graphify-out` mkdirs fail before the normal rebuild error handling
/// can run. Hooks that know the repo root export `GRAPHIFY_REPO_ROOT`, so the
/// rebuild recovers by chdir'ing there. Mirrors graphify-py
/// `_stabilize_rebuild_cwd`; returns `false` (skip the rebuild) when the CWD is
/// gone and no repo root is available.
fn stabilize_rebuild_cwd(watch_path: &Path) -> bool {
    if watch_path.is_absolute() {
        return true;
    }
    if let Ok(root) = std::env::var("GRAPHIFY_REPO_ROOT") {
        let root = root.trim();
        if !root.is_empty() && Path::new(root).is_dir() && std::env::set_current_dir(root).is_ok() {
            return true;
        }
    }
    if std::env::current_dir().is_ok() {
        return true;
    }
    eprintln!(
        "[graphify watch] Rebuild failed: current working directory no longer \
         exists and GRAPHIFY_REPO_ROOT is not set."
    );
    false
}
