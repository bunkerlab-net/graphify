//! GitHub CLI abstraction: `GhClient` trait + process-backed implementation.
//!
//! Tests inject a `FakeGhClient` that returns hard-coded JSON without spawning
//! any real processes.

use std::process::Command;

use crate::error::PrsError;
use crate::model::{CheckRun, PrInfo, parse_ci};

// ── Raw JSON shapes deserialized from `gh` ─────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct RawAuthor {
    login: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u64,
    title: String,
    head_ref_name: String,
    base_ref_name: String,
    author: Option<RawAuthor>,
    is_draft: bool,
    review_decision: Option<String>,
    status_check_rollup: Option<Vec<CheckRun>>,
    updated_at: String,
}

// ── Trait ──────────────────────────────────────────────────────────────────

/// Abstraction over GitHub CLI calls. Implementations may shell out to `gh` or
/// return canned data for tests.
pub trait GhClient {
    /// List open PRs, returning JSON bytes (`gh pr list --json …`).
    ///
    /// # Errors
    ///
    /// Returns `Err(PrsError)` when `gh` is not found, not authenticated, or
    /// returns a non-zero exit code.
    fn pr_list(&self, repo: Option<&str>, limit: usize) -> Result<Vec<u8>, PrsError>;

    /// Return the default branch name (`gh repo view --json defaultBranchRef`).
    fn repo_default_branch(&self, repo: Option<&str>) -> Option<String>;

    /// Return the list of changed file names for a PR (`gh pr diff --name-only`).
    fn pr_files(&self, number: u64, repo: Option<&str>) -> Vec<String>;
}

// ── Process-backed implementation ─────────────────────────────────────────

/// Shells out to the real `gh` CLI.
pub struct ProcessGhClient;

impl GhClient for ProcessGhClient {
    fn pr_list(&self, repo: Option<&str>, limit: usize) -> Result<Vec<u8>, PrsError> {
        let mut cmd = Command::new("gh");
        cmd.args([
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            &limit.to_string(),
            "--json",
            "number,title,headRefName,baseRefName,author,isDraft,\
             reviewDecision,statusCheckRollup,updatedAt",
        ]);
        if let Some(r) = repo {
            cmd.args(["--repo", r]);
        }
        let out = cmd
            .output()
            .map_err(|e| PrsError::GhNotFound(e.to_string()))?;
        if !out.status.success() {
            return Err(PrsError::GhFailed(
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ));
        }
        Ok(out.stdout)
    }

    fn repo_default_branch(&self, repo: Option<&str>) -> Option<String> {
        let mut cmd = Command::new("gh");
        cmd.args(["repo", "view", "--json", "defaultBranchRef"]);
        if let Some(r) = repo {
            cmd.args(["--repo", r]);
        }
        let out = cmd.output().ok()?;
        if !out.status.success() {
            return None;
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        v.get("defaultBranchRef")
            .and_then(|d| d.get("name"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    fn pr_files(&self, number: u64, repo: Option<&str>) -> Vec<String> {
        let mut cmd = Command::new("gh");
        cmd.args(["pr", "diff", &number.to_string(), "--name-only"]);
        if let Some(r) = repo {
            cmd.args(["--repo", r]);
        }
        let Ok(out) = cmd.output() else {
            return vec![];
        };
        if !out.status.success() {
            return vec![];
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }
}

// ── Parsing helpers (shared by both impls) ─────────────────────────────────

/// Parse `gh pr list` JSON bytes into `Vec<PrInfo>`.
///
/// # Errors
///
/// Returns `Err(PrsError)` when the bytes are not valid JSON or a date field
/// cannot be parsed.
pub fn parse_pr_list(bytes: &[u8], expected_base: &str) -> Result<Vec<PrInfo>, PrsError> {
    let raw: Vec<RawPr> = serde_json::from_slice(bytes)?;
    let mut prs = Vec::with_capacity(raw.len());
    for item in raw {
        let updated_at = chrono::DateTime::parse_from_rfc3339(
            // GitHub uses 'Z' suffix — chrono handles that fine.
            &item.updated_at,
        )
        .map_err(|e| PrsError::DateParse(e.to_string()))?
        .to_utc();

        let rollup = item.status_check_rollup.unwrap_or_default();
        prs.push(PrInfo {
            number: item.number,
            title: item.title,
            branch: item.head_ref_name,
            base_branch: item.base_ref_name,
            author: item
                .author
                .and_then(|a| a.login)
                .unwrap_or_else(|| "?".to_string()),
            is_draft: item.is_draft,
            review_decision: item.review_decision.unwrap_or_default(),
            ci_status: parse_ci(&rollup),
            updated_at,
            expected_base: expected_base.to_string(),
            worktree_path: None,
            communities_touched: vec![],
            nodes_affected: 0,
            files_changed: vec![],
        });
    }
    Ok(prs)
}
