//! GitHub PR analysis — `graphify prs` sub-command.
//!
//! Ports `graphify-py/graphify/prs.py`.
//!
//! # Architecture
//!
//! | Module           | Responsibility                                         |
//! |------------------|--------------------------------------------------------|
//! | [`color`]        | ANSI helpers; honours `NO_COLOR` + `IsTerminal`        |
//! | [`model`]        | `PrInfo`, classification, CI parsing, `path_match`     |
//! | [`gh`]           | `GhClient` trait + `ProcessGhClient`                   |
//! | [`git`]          | `GitClient` trait + `ProcessGitClient`                 |
//! | [`graph`]        | Community-impact analysis                              |
//! | [`dashboard`]    | Rendering (`render_*`, `format_prs_text`)              |
//! | [`triage`]       | `TriageBackend` trait + no-op stub                     |
//! | [`error`]        | `PrsError` enum                                        |

pub mod color;
pub mod dashboard;
pub mod error;
pub mod gh;
pub mod git;
pub mod graph;
pub mod model;
pub mod triage;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use dashboard::format_prs_text;
pub use error::PrsError;
pub use model::{PrInfo, classify, parse_ci, path_match};

use gh::{GhClient, parse_pr_list};
use git::{GitClient, branch_from_symbolic_ref, parse_worktree_porcelain};
use graph::{build_community_labels, build_file_index, compute_pr_impact, load_graph_json};
use triage::{TriageBackend, build_triage_prompt};

// ── Default-branch detection ──────────────────────────────────────────────

/// Detect the repo's default branch: gh first, then `git symbolic-ref`, then "main".
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

// ── Fetch helpers ──────────────────────────────────────────────────────────

/// Fetch open PRs from GitHub.
///
/// # Errors
///
/// Returns `Err(PrsError)` when the `gh` CLI is unavailable or returns bad data.
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
/// Returns community labels map `{community_id → [top labels]}`.
pub fn attach_graph_impact(
    prs: &mut [PrInfo],
    graph_path: &Path,
    gh_client: &dyn GhClient,
    repo: Option<&str>,
) -> indexmap::IndexMap<i64, Vec<String>> {
    let Some(data) = load_graph_json(graph_path) else {
        return indexmap::IndexMap::new();
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

// ── Entry point ────────────────────────────────────────────────────────────

/// Argument bag for [`run_cmd_prs`].
///
/// The four boolean flags are a direct port of the Python CLI; they cannot
/// reasonably be collapsed into an enum without losing the independent-flag
/// semantics (e.g. `--triage` can combine with other options).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
pub struct PrsArgs {
    pub base: Option<String>,
    pub repo: Option<String>,
    pub do_triage: bool,
    pub do_worktrees: bool,
    pub do_conflicts: bool,
    pub show_wrong_base: bool,
    pub pr_number: Option<u64>,
    pub graph_path: Option<PathBuf>,
}

impl PrsArgs {
    /// Parse CLI argv slice into `PrsArgs`.
    ///
    /// Returns `None` if `--help` / `-h` was found (caller should print help).
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

/// Run the `prs` sub-command with injected clients.
///
/// # Errors
///
/// Returns `Err(PrsError)` when the `gh` CLI is unavailable or returns bad data.
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

    let mut prs = fetch_prs(gh_client, git_client, repo, Some(&base), 50)?;

    // Attach worktree paths.
    let worktrees = fetch_worktrees(git_client);
    for pr in &mut prs {
        pr.worktree_path = worktrees.get(&pr.branch).cloned();
    }

    // Graph impact (expensive) — only fetch when needed.
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

/// Convenience — re-export `build_community_labels` for callers that already
/// have graph JSON (e.g. MCP tools).
pub use graph::build_community_labels as build_community_labels_from_graph;
