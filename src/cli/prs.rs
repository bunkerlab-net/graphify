//! `prs` command — GitHub PR dashboard.

use anyhow::Result;
use graphify_prs::triage::TriageBackend;

/// `TriageBackend` implementation that calls the configured LLM backend.
///
/// Mirrors Python's `triage_with_opus` at `graphify-py/graphify/prs.py:576`:
/// builds the prompt via `graphify_prs::triage::build_triage_prompt`, calls
/// the LLM with a 1024-token cap, and prints the response indented under
/// the "Triage" header so it matches the Python output shape.
struct LlmTriageBackend;

impl TriageBackend for LlmTriageBackend {
    /// Call the detected LLM backend to triage `candidates` and print the response.
    fn triage(
        &self,
        candidates: &[&graphify_prs::model::PrInfo],
        prompt: &str,
    ) -> Result<(), String> {
        if candidates.is_empty() {
            return Ok(());
        }
        let backend = graphify_llm::detect_backend().ok_or_else(|| {
            "no LLM API key found. Set GEMINI_API_KEY/MOONSHOT_API_KEY/\
                 ANTHROPIC_API_KEY/OPENAI_API_KEY/DEEPSEEK_API_KEY, or run \
                 `claude` once for claude-cli auth."
                .to_string()
        })?;
        println!();
        println!("  Triage ({backend})");
        println!();
        let response = graphify_llm::call_llm(prompt, &backend, 1024)
            .map_err(|e| format!("LLM call failed: {e}"))?;
        // Indent each line by two spaces to match Python's `print("  ", ...)` prefix.
        for line in response.lines() {
            println!("  {line}");
        }
        println!();
        Ok(())
    }
}

/// Run the GitHub PR dashboard, forwarding all CLI flags into [`graphify_prs::PrsArgs`].
///
/// Each flag maps 1:1 to the corresponding field on `PrsArgs`, mirroring Python's
/// `PrsArgs.parse(sys.argv[2:])` call at `__main__.py:1476`.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
// reason: each bool maps 1:1 to a distinct Python CLI flag; collapsing into a
// struct would break the 1:1 parity with PrsArgs.parse(sys.argv[2:]).
pub(crate) fn cmd_prs(
    number: Option<u64>,
    repo: Option<&str>,
    base: Option<&str>,
    limit: usize,
    triage: bool,
    worktrees: bool,
    conflicts: bool,
    wrong_base: bool,
    graph: Option<&std::path::Path>,
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
        graph_path: graph
            .map(std::path::Path::to_path_buf)
            .or_else(|| Some(std::path::PathBuf::from("graphify-out/graph.json"))),
        limit,
    };
    // Only wire the LLM triage backend when the user actually requested triage.
    // Otherwise stay with the no-op so a missing API key never breaks the
    // standalone PR dashboard.
    if triage {
        graphify_prs::run_cmd_prs(
            &graphify_prs::gh::ProcessGhClient,
            &graphify_prs::git::ProcessGitClient,
            &LlmTriageBackend,
            &args,
        )?;
    } else {
        graphify_prs::run_cmd_prs(
            &graphify_prs::gh::ProcessGhClient,
            &graphify_prs::git::ProcessGitClient,
            &graphify_prs::triage::NoOpTriageBackend,
            &args,
        )?;
    }
    Ok(())
}
