//! Argument parsing for the `graphify prs` CLI sub-command.

use std::path::PathBuf;

/// Argument bag for [`crate::run_cmd_prs`].
///
/// The four boolean flags are a direct port of the Python CLI; they
/// cannot reasonably be collapsed into an enum without losing the
/// independent-flag semantics (e.g. `--triage` can combine with other
/// options).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub struct PrsArgs {
    /// Base branch to filter against (`--base` / `-b`).
    pub base: Option<String>,
    /// Repository spec passed to `gh` (`--repo` / `-R`).
    pub repo: Option<String>,
    /// Run the triage backend (`--triage`).
    pub do_triage: bool,
    /// Render the worktrees view (`--worktrees`).
    pub do_worktrees: bool,
    /// Render the conflicts view (`--conflicts`).
    pub do_conflicts: bool,
    /// Include PRs whose base branch doesn't match the detected default
    /// (`--wrong-base`).
    pub show_wrong_base: bool,
    /// Show the detail view for a single PR number (positional or `#N`).
    pub pr_number: Option<u64>,
    /// Path to the graph JSON for community-impact analysis (`--graph`).
    pub graph_path: Option<PathBuf>,
    /// Cap on the number of PRs to fetch (`--limit`).
    pub limit: usize,
}

impl Default for PrsArgs {
    /// Return [`PrsArgs`] with all flags unset and a default `limit` of 50.
    fn default() -> Self {
        Self {
            base: None,
            repo: None,
            do_triage: false,
            do_worktrees: false,
            do_conflicts: false,
            show_wrong_base: false,
            pr_number: None,
            graph_path: None,
            limit: 50,
        }
    }
}

impl PrsArgs {
    /// Parse a CLI `argv` slice into [`PrsArgs`].
    ///
    /// Returns `None` if `--help` / `-h` was found; the caller should
    /// print help text in that case.
    ///
    /// Default `graph_path` is `graphify-out/graph.json`.
    #[must_use]
    pub fn parse(argv: &[&str]) -> Option<Self> {
        #[allow(clippy::similar_names)] // `argv` param vs `parsed` local is intentional
        let mut parsed = Self {
            graph_path: Some(PathBuf::from("graphify-out/graph.json")),
            ..Self::default()
        };
        let mut i = 0_usize;
        while i < argv.len() {
            match argv[i] {
                "--triage" => parsed.do_triage = true,
                "--worktrees" => parsed.do_worktrees = true,
                "--conflicts" => parsed.do_conflicts = true,
                "--wrong-base" => parsed.show_wrong_base = true,
                "-h" | "--help" => return None,
                "--base" | "-b" if i + 1 < argv.len() => {
                    parsed.base = Some(argv[i + 1].to_string());
                    i += 1;
                }
                arg if arg.starts_with("--base=") => {
                    parsed.base = Some(arg["--base=".len()..].to_string());
                }
                "--repo" | "-R" if i + 1 < argv.len() => {
                    parsed.repo = Some(argv[i + 1].to_string());
                    i += 1;
                }
                arg if arg.starts_with("--graph=") => {
                    parsed.graph_path = Some(PathBuf::from(&arg["--graph=".len()..]));
                }
                "--graph" if i + 1 < argv.len() => {
                    parsed.graph_path = Some(PathBuf::from(argv[i + 1]));
                    i += 1;
                }
                "--limit" if i + 1 < argv.len() => {
                    if let Ok(n) = argv[i + 1].parse::<usize>() {
                        parsed.limit = n;
                    }
                    i += 1;
                }
                arg if arg.starts_with("--limit=") => {
                    if let Ok(n) = arg["--limit=".len()..].parse::<usize>() {
                        parsed.limit = n;
                    }
                }
                arg => {
                    let stripped = arg.trim_start_matches('#');
                    if !stripped.is_empty()
                        && stripped.chars().all(|c| c.is_ascii_digit())
                        && let Ok(n) = stripped.parse::<u64>()
                    {
                        parsed.pr_number = Some(n);
                    }
                }
            }
            i += 1;
        }
        Some(parsed)
    }
}
