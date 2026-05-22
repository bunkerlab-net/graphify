//! `prs` command — GitHub PR dashboard.

use anyhow::Result;

/// Run the GitHub PR dashboard, forwarding all CLI flags into [`graphify_prs::PrsArgs`].
///
/// Each flag maps 1:1 to the corresponding field on `PrsArgs`, mirroring Python's
/// `PrsArgs.parse(sys.argv[2:])` call at `__main__.py:1476`.  The `--limit` flag
/// is accepted for CLI consistency but `run_cmd_prs` currently uses an internal
/// hard-coded fetch limit of 50; this is a known gap in the crate API.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
// reason: each bool maps 1:1 to a distinct Python CLI flag; collapsing into a
// struct would break the 1:1 parity with PrsArgs.parse(sys.argv[2:]).
pub(crate) fn cmd_prs(
    number: Option<u64>,
    repo: Option<&str>,
    base: Option<&str>,
    _limit: usize,
    triage: bool,
    worktrees: bool,
    conflicts: bool,
    wrong_base: bool,
) -> Result<()> {
    eprintln!(
        "fetching PRs{} via gh CLI ...",
        repo.map(|r| format!(" for {r}")).unwrap_or_default()
    );
    let args = graphify_prs::PrsArgs {
        repo: repo.map(str::to_string),
        base: base.map(str::to_string),
        pr_number: number,
        do_triage: triage,
        do_worktrees: worktrees,
        do_conflicts: conflicts,
        show_wrong_base: wrong_base,
        ..Default::default()
    };
    graphify_prs::run_cmd_prs(
        &graphify_prs::gh::ProcessGhClient,
        &graphify_prs::git::ProcessGitClient,
        &graphify_prs::triage::NoOpTriageBackend,
        &args,
    )?;
    Ok(())
}
