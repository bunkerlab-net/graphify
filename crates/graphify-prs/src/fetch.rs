//! PR / worktree / graph-impact fetching helpers.

use std::collections::HashMap;
use std::path::Path;

use indexmap::IndexMap;

use crate::detect::detect_default_branch;
use crate::error::PrsError;
use crate::gh::{GhClient, parse_pr_list};
use crate::git::{GitClient, parse_worktree_porcelain};
use crate::graph::{build_community_labels, build_file_index, compute_pr_impact, load_graph_json};
use crate::model::PrInfo;

/// Fetch open PRs from GitHub.
///
/// `base` defaults to the repo's detected default branch when `None`.
///
/// # Errors
///
/// Returns [`PrsError`] when the `gh` CLI is unavailable or returns bad
/// data.
pub fn fetch_prs(
    gh_client: &dyn GhClient,
    git_client: &dyn GitClient,
    repo: Option<&str>,
    base: Option<&str>,
    limit: usize,
) -> Result<Vec<PrInfo>, PrsError> {
    let resolved_base = base.map_or_else(
        || detect_default_branch(gh_client, git_client, repo),
        str::to_string,
    );

    let bytes = gh_client.pr_list(repo, limit)?;
    parse_pr_list(&bytes, &resolved_base)
}

/// Parse `git worktree list --porcelain` output into `{branch → path}`.
///
/// Delegates to [`GitClient`]; returns an empty map on failure.
#[must_use]
pub fn fetch_worktrees(git_client: &dyn GitClient) -> HashMap<String, String> {
    git_client
        .worktree_list_porcelain()
        .map(|text| parse_worktree_porcelain(&text))
        .unwrap_or_default()
}

/// Attach graph-impact data to each PR in-place.
///
/// Returns the community labels map
/// `{community_id → [top labels]}` so callers can render summaries.
/// PRs whose status is `WRONG-BASE` are skipped (their files aren't
/// fetched).
pub fn attach_graph_impact(
    prs: &mut [PrInfo],
    graph_path: &Path,
    gh_client: &dyn GhClient,
    repo: Option<&str>,
) -> IndexMap<i64, Vec<String>> {
    let Some(data) = load_graph_json(graph_path) else {
        return IndexMap::new();
    };

    let nodes = data
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let index = build_file_index(&nodes);

    for pr in prs.iter_mut() {
        if pr.status() == "WRONG-BASE" {
            continue;
        }
        let files = gh_client.pr_files(pr.number, repo);
        pr.files_changed = files;
        let (comms, nodes_affected) = compute_pr_impact(&pr.files_changed, &index);
        pr.communities_touched = comms;
        pr.nodes_affected = nodes_affected;
    }

    build_community_labels(&data, 4)
}
