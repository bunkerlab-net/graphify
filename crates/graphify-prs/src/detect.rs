//! Default-branch detection.

use crate::gh::GhClient;
use crate::git::{GitClient, branch_from_symbolic_ref};

/// Detect the repository's default branch.
///
/// Tries in order:
/// 1. `gh repo view --json defaultBranchRef` via [`GhClient`].
/// 2. `git symbolic-ref refs/remotes/origin/HEAD` via [`GitClient`].
/// 3. The string `"main"` as a final fallback.
#[must_use]
pub fn detect_default_branch(
    gh_client: &dyn GhClient,
    git_client: &dyn GitClient,
    repo: Option<&str>,
) -> String {
    if let Some(branch) = gh_client.repo_default_branch(repo) {
        return branch;
    }
    if let Some(symbolic) = git_client.symbolic_ref_origin_head()
        && let Some(branch) = branch_from_symbolic_ref(&symbolic)
    {
        return branch;
    }
    "main".to_string()
}
