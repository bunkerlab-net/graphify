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

/// Run the GitHub PR dashboard from a parsed [`graphify_prs::PrsArgs`] bag.
///
/// Mirrors Python's `PrsArgs.parse(sys.argv[2:])` call at `__main__.py:1476`.
pub(crate) fn cmd_prs(args: &graphify_prs::PrsArgs) -> Result<()> {
    eprintln!(
        "fetching PRs{} via gh CLI ...",
        args.repo
            .as_deref()
            .map(|r| format!(" for {r}"))
            .unwrap_or_default()
    );
    // Only wire the LLM triage backend when the user actually requested triage.
    // Otherwise stay with the no-op so a missing API key never breaks the
    // standalone PR dashboard.
    if args.do_triage {
        graphify_prs::run_cmd_prs(
            &graphify_prs::gh::ProcessGhClient,
            &graphify_prs::git::ProcessGitClient,
            &LlmTriageBackend,
            args,
        )?;
    } else {
        graphify_prs::run_cmd_prs(
            &graphify_prs::gh::ProcessGhClient,
            &graphify_prs::git::ProcessGitClient,
            &graphify_prs::triage::NoOpTriageBackend,
            args,
        )?;
    }
    Ok(())
}
