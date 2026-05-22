//! Git helper for querying the current HEAD commit hash.
//!
//! Extracted from `rebuild.rs` so that the git interaction lives in one
//! focused file independent of the wider pipeline.

use std::path::Path;

/// Return the current HEAD commit hash, or `None` when not inside a git repo.
///
/// Runs `git rev-parse HEAD` in the current working directory.  Failure for
/// any reason (no git, not a repo, no commits yet) is silently swallowed and
/// returns `None`, matching Python's `except Exception: return None`.
///
/// Ports `_git_head` from `watch.py:102-109`.
#[must_use]
pub fn git_head(path: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8(output.stdout).ok()?;
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        None
    }
}
