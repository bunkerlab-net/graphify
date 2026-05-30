//! `#1059` pending-changes queue.
//!
//! A post-commit hook process that cannot acquire the rebuild lock appends its
//! change set to `<out>/.pending_changes` so the work is not silently dropped
//! under lock contention. The lock-holding process drains the queue and merges
//! it into its own rebuild. Ports `_queue_pending` / `_drain_pending` /
//! `_merge_changed_paths` from `graphify-py/graphify/watch.py`.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::WatchError;

/// Filename of the pending-changes queue inside the graphify-out directory.
pub const PENDING_FILENAME: &str = ".pending_changes";

/// Maximum late-arrival drain passes before quiescing, so a storm of commits
/// eventually settles without livelocking the lock-holder.
pub const PENDING_DRAIN_MAX_PASSES: usize = 20;

/// Append `changed_paths` to `<out>/.pending_changes`, one per line.
///
/// Opened in append mode so concurrent writers do not clobber each other's
/// existing content on POSIX. `write_all` may still issue more than one `write`
/// syscall, so two concurrent writers can interleave mid-line for large
/// payloads; line-atomicity here relies on the typically small payload landing
/// in a single sub-`PIPE_BUF` write, and the drain pass tolerates a torn line by
/// skipping it. A no-op when `changed_paths` is empty, so an empty change set
/// never creates an empty queue file.
///
/// # Errors
/// Propagates filesystem errors via [`WatchError`].
pub fn queue_pending(out: &Path, changed_paths: &[PathBuf]) -> Result<(), WatchError> {
    if changed_paths.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(out)?;
    let pending = out.join(PENDING_FILENAME);
    // A trailing newline is always written so a torn partial line stays
    // confined to the offending entry and is skipped on drain.
    let mut payload = String::new();
    for p in changed_paths {
        payload.push_str(&p.to_string_lossy());
        payload.push('\n');
    }
    let mut fh = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&pending)?;
    fh.write_all(payload.as_bytes())?;
    Ok(())
}

/// Read + unlink `<out>/.pending_changes`, returning the deduplicated paths in
/// first-seen order.
///
/// Empty/whitespace-only lines are skipped so a partial concurrent write that
/// left only a fragment cannot poison the merge. Returns an empty vec when the
/// file does not exist. Unlinks *after* reading: losing the file post-read is
/// fine (the data is in the returned vec); losing it before would be a bug.
#[must_use]
pub fn drain_pending(out: &Path) -> Vec<PathBuf> {
    let pending = out.join(PENDING_FILENAME);
    let Ok(raw) = fs::read_to_string(&pending) else {
        return Vec::new();
    };
    // Tolerate a racing drain that already removed the file.
    let _ = fs::remove_file(&pending);
    let mut seen: HashSet<String> = HashSet::new();
    let mut out_paths: Vec<PathBuf> = Vec::new();
    for line in raw.lines() {
        let s = line.trim();
        if s.is_empty() || !seen.insert(s.to_string()) {
            continue;
        }
        out_paths.push(PathBuf::from(s));
    }
    out_paths
}

/// Concatenate path lists, preserving first-seen order and dropping duplicates.
///
/// Used to combine a hook process's own `changed_paths` with the drained queue
/// so a single rebuild covers every queued commit's worth of files (#1059).
#[must_use]
pub fn merge_changed_paths(sources: &[Option<&[PathBuf]>]) -> Vec<PathBuf> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for src in sources.iter().flatten() {
        for p in *src {
            if seen.insert(p.to_string_lossy().into_owned()) {
                out.push(p.clone());
            }
        }
    }
    out
}

/// Drive the lock-holding rebuild step: drain anything queued by earlier
/// contenders (including the paths we queued ourselves), merge with our own
/// change set, run the rebuild via `run_inner`, then loop to drain late
/// arrivals queued mid-rebuild (#1059).
///
/// `run_inner` is the rebuild callback — production passes the lock-free
/// `rebuild_code_inner`; tests inject a recording closure so the merge and
/// late-drain orchestration is unit-testable without spawning the real
/// pipeline. A full-corpus rebuild (`changed_paths == None`) supersedes any
/// queued incremental work: it drains the queue on entry but skips the
/// late-arrival loop because it already covers every file.
///
/// # Errors
/// Propagates whatever error `run_inner` returns.
pub fn rebuild_with_pending<F>(
    out: &Path,
    changed_paths: Option<&[PathBuf]>,
    mut run_inner: F,
) -> Result<bool, WatchError>
where
    F: FnMut(Option<&[PathBuf]>) -> Result<bool, WatchError>,
{
    let merged: Option<Vec<PathBuf>> = if let Some(own) = changed_paths {
        let drained = drain_pending(out);
        Some(merge_changed_paths(&[Some(own), Some(&drained)]))
    } else {
        // Full-corpus rebuild supersedes any queued incremental work.
        let _ = drain_pending(out);
        None
    };

    let mut ok = run_inner(merged.as_deref())?;

    if merged.is_some() {
        for _ in 0..PENDING_DRAIN_MAX_PASSES {
            let late = drain_pending(out);
            if late.is_empty() {
                break;
            }
            ok = run_inner(Some(&late))? && ok;
        }
    }
    Ok(ok)
}
