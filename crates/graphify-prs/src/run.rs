//! Top-level CLI driver for the `prs` sub-command.

use std::path::PathBuf;

use crate::args::PrsArgs;
use crate::color;
use crate::dashboard;
use crate::detect::detect_default_branch;
use crate::error::PrsError;
use crate::fetch::{attach_graph_impact, fetch_prs, fetch_worktrees};
use crate::gh::GhClient;
use crate::git::GitClient;
use crate::model::PrInfo;
use crate::triage::{TriageBackend, build_triage_prompt};

/// Run the `prs` sub-command with injected clients.
///
/// Drives base-branch detection, PR fetching, worktree annotation,
/// optional graph-impact analysis, and dispatches to the requested
/// dashboard / detail / worktrees / conflicts / triage view.
///
/// # Errors
///
/// Returns [`PrsError`] when the `gh` CLI is unavailable, returns bad
/// data, or [`PrsError::PrNotFound`] when `args.pr_number` does not
/// match any open PR.
pub fn run_cmd_prs(
    gh_client: &dyn GhClient,
    git_client: &dyn GitClient,
    triage_backend: &dyn TriageBackend,
    args: &PrsArgs,
) -> Result<(), PrsError> {
    let repo = args.repo.as_deref();
    let base = args
        .base
        .clone()
        .unwrap_or_else(|| detect_default_branch(gh_client, git_client, repo));

    let mut prs = fetch_prs(gh_client, git_client, repo, Some(&base), args.limit)?;

    let worktrees = fetch_worktrees(git_client);
    for pr in &mut prs {
        pr.worktree_path = worktrees.get(&pr.branch).cloned();
    }

    let graph_path = args
        .graph_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("graphify-out/graph.json"));
    let needs_impact =
        graph_path.exists() && (args.pr_number.is_some() || args.do_triage || args.do_conflicts);
    let community_labels = if needs_impact {
        attach_graph_impact(&mut prs, &graph_path, gh_client, repo)
    } else {
        indexmap::IndexMap::new()
    };

    if let Some(n) = args.pr_number {
        let pr = prs
            .iter()
            .find(|p| p.number == n)
            .ok_or(PrsError::PrNotFound(n))?;
        dashboard::render_pr_detail(pr);
        return Ok(());
    }

    if args.do_triage {
        dashboard::render_dashboard(&prs, &base, args.show_wrong_base);
        let candidates: Vec<&PrInfo> = prs
            .iter()
            .filter(|p| {
                p.base_branch == base && p.status() != "WRONG-BASE" && p.status() != "STALE"
            })
            .collect();
        if candidates.is_empty() {
            println!("{}", color::dim("  No actionable PRs to triage."));
        } else {
            let prompt = build_triage_prompt(&candidates);
            if let Err(e) = triage_backend.triage(&candidates, &prompt) {
                eprintln!("{}", color::red(&format!("  Triage failed: {e}")));
            }
        }
        return Ok(());
    }

    if args.do_worktrees {
        dashboard::render_worktrees(&prs, &worktrees);
        return Ok(());
    }

    if args.do_conflicts {
        dashboard::render_dashboard(&prs, &base, args.show_wrong_base);
        dashboard::render_conflicts(&prs, &base, Some(&community_labels));
        return Ok(());
    }

    dashboard::render_dashboard(&prs, &base, args.show_wrong_base);
    Ok(())
}
