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
pub use shrink::{ShrinkChecker, check_shrink};

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
    rebuild_code_impl(watch_path, changed_paths, opts, check_shrink)
}

/// [`rebuild_code`] with an injectable shrink-guard. The public entry always
/// supplies the real [`check_shrink`]; `test_support` supplies a rejecting
/// checker scoped to a single call, so no global state or environment variable
/// can alter production behaviour.
pub(crate) fn rebuild_code_impl(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    opts: RebuildOptions,
    check_shrink_fn: ShrinkChecker,
) -> Result<bool, WatchError> {
    let Some(effective) = effective_watch_path(watch_path) else {
        return Ok(false);
    };
    let out = effective.join(graphify_security::graphify_out());

    match opts.lock {
        LockPolicy::None => rebuild_code_inner(
            &effective,
            changed_paths,
            opts.force,
            opts.no_cluster,
            opts.follow_symlinks,
            check_shrink_fn,
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
                    &effective,
                    paths,
                    opts.force,
                    opts.no_cluster,
                    opts.follow_symlinks,
                    check_shrink_fn,
                )
            });
            drop(guard);
            result
        }
    }
}

/// Resolve the watch path to use for a rebuild WITHOUT mutating the process
/// working directory.
///
/// Detached git hooks can inherit a transient working directory that is deleted
/// before the background rebuild starts; in that state `current_dir()` and the
/// relative `graphify-out` mkdirs fail. Rather than `chdir`-ing (which mutates
/// process-global state shared by any concurrent caller), this resolves the
/// path: a valid CWD keeps the caller-supplied (often relative) path as-is so
/// the committed `.graphify_root` marker stays portable (#777); only when the
/// CWD is gone does it fall back to rooting the path under `GRAPHIFY_REPO_ROOT`.
/// Returns `None` (skip the rebuild) when neither is available.
///
/// Divergence from graphify-py `_stabilize_rebuild_cwd`: the reference `chdir`s
/// to `GRAPHIFY_REPO_ROOT` unconditionally; here it is a CWD-gone fallback so a
/// valid working directory is preferred (git hooks run from the repo root, so
/// the two agree) and the process CWD is never disturbed.
fn effective_watch_path(watch_path: &Path) -> Option<PathBuf> {
    if watch_path.is_absolute() {
        return Some(watch_path.to_path_buf());
    }
    if std::env::current_dir().is_ok() {
        return Some(watch_path.to_path_buf());
    }
    if let Ok(root) = std::env::var("GRAPHIFY_REPO_ROOT") {
        let root = root.trim();
        if !root.is_empty() && Path::new(root).is_dir() {
            return Some(Path::new(root).join(watch_path));
        }
    }
    eprintln!(
        "[graphify watch] Rebuild failed: current working directory no longer \
         exists and GRAPHIFY_REPO_ROOT is not set."
    );
    None
}
