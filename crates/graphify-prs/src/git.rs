//! Git CLI abstraction: `GitClient` trait + process-backed implementation.
//!
//! Currently covers:
//!  - `git worktree list --porcelain` → `{branch: worktree_path}` map
//!  - `git symbolic-ref refs/remotes/origin/HEAD` → default branch fallback

use std::collections::HashMap;
use std::process::Command;

// ── Trait ──────────────────────────────────────────────────────────────────

/// Abstraction over Git CLI calls.
pub trait GitClient {
    /// Run `git worktree list --porcelain` and return the raw output string.
    fn worktree_list_porcelain(&self) -> Option<String>;

    /// Run `git symbolic-ref refs/remotes/origin/HEAD` and return trimmed stdout.
    fn symbolic_ref_origin_head(&self) -> Option<String>;
}

// ── Process-backed implementation ─────────────────────────────────────────

/// Shells out to the real `git` binary.
pub struct ProcessGitClient;

impl GitClient for ProcessGitClient {
    /// Invokes `git worktree list --porcelain` and returns raw output.
    fn worktree_list_porcelain(&self) -> Option<String> {
        let out = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Invokes `git symbolic-ref refs/remotes/origin/HEAD` and returns trimmed output.
    fn symbolic_ref_origin_head(&self) -> Option<String> {
        let out = Command::new("git")
            .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

// ── Parsing ────────────────────────────────────────────────────────────────

/// Parse `git worktree list --porcelain` output into `{branch → path}`.
///
/// A blank line is the record separator; it resets state so that a detached
/// HEAD entry doesn't leak its path into the next record's branch.
#[must_use]
pub fn parse_worktree_porcelain(text: &str) -> HashMap<String, String> {
    let mut mapping: HashMap<String, String> = HashMap::new();
    let mut current_path: Option<String> = None;

    for line in text.lines() {
        if line.is_empty() {
            // Record separator — reset to avoid leaking across detached HEADs.
            current_path = None;
        } else if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.to_string());
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/")
            && let Some(ref p) = current_path
        {
            mapping.insert(branch.to_string(), p.clone());
        }
    }
    mapping
}

/// Parse the symbolic-ref output into just the branch name.
///
/// `"refs/remotes/origin/main\n"` → `"main"`.
#[must_use]
pub fn branch_from_symbolic_ref(r: &str) -> Option<String> {
    let trimmed = r.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.split('/').next_back()?.to_string())
}
