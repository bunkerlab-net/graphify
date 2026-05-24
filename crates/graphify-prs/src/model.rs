//! Core data model: `PrInfo` struct, classification, CI-status parsing.

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Status classification order (matches Python `_STATUS_ORDER`).
pub const STATUS_ORDER: &[&str] = &[
    "WRONG-BASE",
    "CI-FAIL",
    "CHANGES-REQ",
    "DRAFT",
    "STALE",
    "PENDING",
    "APPROVED",
    "READY",
];

/// Days before a PR is considered stale (matches Python `_STALE_DAYS`).
pub const STALE_DAYS: i64 = 14;

/// A single open pull request fetched from the GitHub CLI.
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub branch: String,
    pub base_branch: String,
    pub author: String,
    pub is_draft: bool,
    /// `APPROVED` | `CHANGES_REQUESTED` | `""`.
    pub review_decision: String,
    /// `SUCCESS` | `FAILURE` | `PENDING` | `NONE`.
    pub ci_status: String,
    pub updated_at: DateTime<Utc>,
    /// Detected or provided default branch for this repo.
    pub expected_base: String,
    /// Path to the local worktree for this branch, if any.
    pub worktree_path: Option<String>,
    /// Graph community IDs touched by this PR.
    pub communities_touched: Vec<i64>,
    /// Number of graph nodes affected.
    pub nodes_affected: usize,
    /// Files changed in this PR.
    pub files_changed: Vec<String>,
}

impl PrInfo {
    /// Classification string for this PR against `expected_base`.
    #[must_use]
    pub fn status(&self) -> String {
        classify(self, &self.expected_base)
    }

    /// Age in whole days since last update.
    #[must_use]
    pub fn days_old(&self) -> i64 {
        (Utc::now() - self.updated_at).num_days()
    }

    /// Human-readable graph blast radius, e.g. `"3 nodes / 2 communities"`.
    #[must_use]
    pub fn blast_radius(&self) -> String {
        if self.nodes_affected == 0 {
            return String::new();
        }
        let n = self.nodes_affected;
        let c = self.communities_touched.len();
        let node_str = if n == 1 { "node" } else { "nodes" };
        let comm_str = if c == 1 { "community" } else { "communities" };
        format!("{n} {node_str} / {c} {comm_str}")
    }
}

/// Classify a PR against the given base branch.
#[must_use]
pub fn classify(pr: &PrInfo, base: &str) -> String {
    if pr.base_branch != base {
        return "WRONG-BASE".to_string();
    }
    if pr.ci_status == "FAILURE" {
        return "CI-FAIL".to_string();
    }
    if pr.review_decision == "CHANGES_REQUESTED" {
        return "CHANGES-REQ".to_string();
    }
    if pr.is_draft {
        return "DRAFT".to_string();
    }
    if pr.days_old() >= STALE_DAYS {
        return "STALE".to_string();
    }
    if pr.review_decision == "APPROVED" {
        return "APPROVED".to_string();
    }
    if pr.ci_status == "PENDING" {
        return "PENDING".to_string();
    }
    "READY".to_string()
}

/// CI failure conclusion strings (matches Python `_CI_FAILURE_CONCLUSIONS`).
const CI_FAILURE_CONCLUSIONS: &[&str] = &[
    "FAILURE",
    "CANCELLED",
    "TIMED_OUT",
    "ACTION_REQUIRED",
    "STARTUP_FAILURE",
];

/// Raw check-run entry from `statusCheckRollup`.
#[derive(Debug, Deserialize)]
pub struct CheckRun {
    /// Terminal conclusion of the check run (e.g. `"SUCCESS"`, `"FAILURE"`).
    /// `None` while the run is still in progress.
    pub conclusion: Option<String>,
    /// Lifecycle status of the check run (e.g. `"IN_PROGRESS"`, `"QUEUED"`, `"COMPLETED"`).
    pub status: Option<String>,
}

/// Derive a single CI status string from a list of check runs.
#[must_use]
pub fn parse_ci(rollup: &[CheckRun]) -> String {
    if rollup.is_empty() {
        return "NONE".to_string();
    }
    let has_failure = rollup.iter().any(|r| {
        r.conclusion
            .as_deref()
            .is_some_and(|c| CI_FAILURE_CONCLUSIONS.contains(&c))
    });
    if has_failure {
        return "FAILURE".to_string();
    }
    let in_progress = rollup.iter().any(|r| {
        r.status
            .as_deref()
            .is_some_and(|s| s == "IN_PROGRESS" || s == "QUEUED")
    });
    if in_progress {
        return "PENDING".to_string();
    }
    let has_success = rollup
        .iter()
        .any(|r| r.conclusion.as_deref() == Some("SUCCESS"));
    if has_success {
        return "SUCCESS".to_string();
    }
    "NONE".to_string()
}

/// True when `graph_src` and `pr_file` refer to the same file (path-boundary safe).
#[must_use]
pub fn path_match(graph_src: &str, pr_file: &str) -> bool {
    if graph_src == pr_file {
        return true;
    }
    let suffix = format!("/{pr_file}");
    if graph_src.ends_with(&suffix) {
        return true;
    }
    let suffix2 = format!("/{graph_src}");
    pr_file.ends_with(&suffix2)
}
