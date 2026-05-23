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

mod args;
pub mod color;
pub mod dashboard;
mod detect;
pub mod error;
mod fetch;
pub mod gh;
pub mod git;
pub mod graph;
pub mod model;
mod run;
pub mod triage;

pub use args::PrsArgs;
pub use dashboard::format_prs_text;
pub use detect::detect_default_branch;
pub use error::PrsError;
pub use fetch::{attach_graph_impact, fetch_prs, fetch_worktrees};
pub use model::{PrInfo, classify, parse_ci, path_match};
pub use run::run_cmd_prs;

/// Convenience — re-export `build_community_labels` for callers that
/// already have graph JSON (e.g. MCP tools).
pub use graph::build_community_labels as build_community_labels_from_graph;
