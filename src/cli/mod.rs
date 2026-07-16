//! CLI module declarations and shared utilities.
//!
//! Each submodule owns the handler function(s) for the corresponding command
//! group.  Shared helpers used by more than one command live here.
//!
//! The `outln!`/`out!` macros defined here (before the submodule declarations,
//! so they are in textual scope for every submodule) are broken-pipe-guarded
//! replacements for `println!`/`print!` (#1807); see [`output`].

/// `println!` that survives a downstream reader closing the pipe early (#1807):
/// a closed-pipe write exits 0, any other write error exits 1. See [`output`].
macro_rules! outln {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        if let ::std::result::Result::Err(e) = writeln!(::std::io::stdout(), $($arg)*) {
            $crate::cli::output::handle_stdout_error(&e);
        }
    }};
}

/// `print!` counterpart of [`outln!`] — guarded against an early pipe close.
///
/// No bare `print!` call site exists today, but this completes the guarded
/// output API so a future partial-line write reaches for the guard instead of
/// the unguarded `print!` and silently reintroducing #1807.
#[allow(unused_macros)] // paired guard for `outln!`; see doc above.
macro_rules! out {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        if let ::std::result::Result::Err(e) = write!(::std::io::stdout(), $($arg)*) {
            $crate::cli::output::handle_stdout_error(&e);
        }
    }};
}

pub(crate) mod add;
pub(crate) mod affected;
pub(crate) mod args;
pub(crate) mod benchmark;
pub(crate) mod cache_check;
pub(crate) mod clone;
pub(crate) mod cluster_only;
pub(crate) mod diagnose;
pub(crate) mod dispatch;
pub(crate) mod export;
pub(crate) mod extract;
pub(crate) mod global;
pub(crate) mod hooks;
pub(crate) mod install;
pub(crate) mod merge;
pub(crate) mod merge_chunks;
pub(crate) mod output;
pub(crate) mod provider;
pub(crate) mod prs;
pub(crate) mod query;
pub(crate) mod reflect;
pub(crate) mod save_result;
pub(crate) mod serve;
pub(crate) mod timer;
pub(crate) mod tree;
pub(crate) mod validate;
pub(crate) mod watch;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

/// Top-level commands for which the startup skill-version check is suppressed:
/// install / uninstall re-stamp the skill, and the hook gates run on every
/// editor tool use and must stay silent (#1568). Mirrors graphify-py's
/// `_silent_cmds`, but matched at a command POSITION (see [`is_silent_invocation`]).
const SKILL_CHECK_SILENT_CMDS: [&str; 4] = ["install", "uninstall", "hook-check", "hook-guard"];

/// `true` when this invocation is a skill-silent command: a top-level
/// [`SKILL_CHECK_SILENT_CMDS`] entry, or a platform group's `install`/`uninstall`
/// subcommand (`graphify claude install`). Positions are validated against clap's
/// command tree so a free-text argument to another command
/// (`graphify query install`) is not mistaken for the `install` command (#1568).
/// `Cli` has no global flags, so argv[1] is always the command token.
fn is_silent_invocation(raw_args: &[String]) -> bool {
    use clap::CommandFactory;
    let Some(top) = raw_args.get(1).map(String::as_str) else {
        return false;
    };
    if SKILL_CHECK_SILENT_CMDS.contains(&top) {
        return true;
    }
    let Some(second) = raw_args.get(2).map(String::as_str) else {
        return false;
    };
    if !matches!(second, "install" | "uninstall") {
        return false;
    }
    // Only a real platform group (`claude`, `gemini`, ...) has an `install`/
    // `uninstall` child; `query`/`extract`/... do not.
    args::Cli::command()
        .find_subcommand(top)
        .is_some_and(|sub| sub.find_subcommand(second).is_some())
}

/// Configure runtime services, parse argv, and dispatch the selected subcommand.
///
/// Holds a `graphify-cache` [`StatIndexFlushGuard`] for the whole call so the
/// stat index is flushed when `run` returns, initialises `tracing`, then parses
/// [`args::Cli`] and forwards to [`dispatch::dispatch`]. When no subcommand is
/// supplied, prints a help hint and returns `Ok(())`. The guard is a scope
/// guard, not a true `atexit` hook: clap's `--help`/`--version`/parse-error
/// paths call `std::process::exit` and bypass it, but those never mutate the
/// index, so nothing is lost.
///
/// [`StatIndexFlushGuard`]: graphify_cache::StatIndexFlushGuard
pub(crate) fn run() -> Result<()> {
    // Hold the flush guard for the whole run so the stat index is persisted on
    // return (a `static` guard would never drop at process exit).
    let _stat_index_flush = graphify_cache::StatIndexFlushGuard::new();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // #1568: warn about a stale on-disk skill (a `.graphify_version` stamp that
    // mismatches this package). Suppressed for the commands that re-stamp the skill
    // (install/uninstall) or run silently on every editor tool use
    // (hook-check/hook-guard) - see `is_silent_invocation`. Checked on raw argv so
    // `--version`/`--help` still surface the warning before clap intercepts them.
    if !is_silent_invocation(&std::env::args().collect::<Vec<_>>()) {
        graphify_hooks::check_skill_versions(env!("CARGO_PKG_VERSION"));
    }

    let parsed = args::Cli::parse();
    match parsed.command {
        None => {
            outln!("graphify {} — run with --help", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(cmd) => dispatch::dispatch(cmd),
    }
}

/// Return the graphify output directory, honouring the `GRAPHIFY_OUT` env var.
///
/// Thin re-export of [`graphify_security::graphify_out`] — the single source of
/// truth for the output-dir override (Python `graphify.paths`). Accepts a
/// relative name (`graphify-out-feature`) or an absolute path
/// (`/shared/graphify-out`).
pub(crate) fn graphify_out_dir() -> PathBuf {
    graphify_security::graphify_out()
}

/// Return the default `graph.json` path, honouring `GRAPHIFY_OUT`.
pub(crate) fn default_graph_path() -> PathBuf {
    graphify_security::default_graph_json()
}

/// Load and parse `graph.json` into a [`graphify_build::Graph`].
///
/// Reads the file, parses JSON, and calls `build_from_json`. Used by every
/// command that needs to traverse or query the graph. Rejects graph files
/// larger than [`graphify_security::MAX_GRAPH_FILE_BYTES`] before reading
/// them into memory — mirrors the Python `_enforce_graph_size_cap_or_exit`
/// helper in `graphify-py/graphify/__main__.py`.
pub(crate) fn load_graph(path: &std::path::Path) -> anyhow::Result<graphify_build::Graph> {
    graphify_security::check_graph_file_size_cap(path)?;
    let contents = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    // #1504: nudge read-only consumers to rebuild a pre-migration graph, since
    // they don't re-extract. Inspect the raw nodes before `build_from_json` moves
    // `value`. Divergence from graphify-py (which wires this into `query` only):
    // every consumer sharing `load_graph` (query/path/explain/export/tree/
    // cluster-only) reads a legacy graph without re-extracting, so the rebuild
    // advice applies equally. Only fires when legacy IDs are actually detected,
    // so freshly-built graphs (and test fixtures) stay silent.
    if let Some(nodes) = value.get("nodes").and_then(serde_json::Value::as_array)
        && graphify_build::graph_has_legacy_ids(nodes, None)
    {
        eprintln!(
            "[graphify] note: this graph uses the pre-#1504 node-ID scheme; \
             rebuild with `graphify extract --force` to get path-qualified IDs \
             (fixes same-name-file collisions)."
        );
    }
    let mut graph = graphify_build::build_from_json(value, true, None)?;
    // Work-memory overlay (#1441): stash the learned-verdict sidecar (if present)
    // next to graph.json onto the graph so read surfaces (explain, query text)
    // can annotate nodes. Best-effort; graph.json itself stays purely structural.
    let overlay: serde_json::Map<String, serde_json::Value> =
        graphify_reflect::load_learning_overlay(path)
            .into_iter()
            .collect();
    graph.graph_attrs.insert(
        "_learning_overlay".to_string(),
        serde_json::Value::Object(overlay),
    );
    Ok(graph)
}

/// Build the analysis JSON consumed by `graphify_report::write_report`.
///
/// Writes both the Python-compatible keys (`cohesion`, `gods`, `surprises`,
/// `tokens`) and the Rust report consumer's preferred aliases
/// (`cohesion_scores`, `god_nodes`, `surprising_connections`,
/// `suggested_questions`).  `graphify_report` reads the alias forms, and
/// `graphify export wiki/obsidian/svg/html` plus the Python pipeline read
/// the Python forms — emitting both keeps cross-version sidecars
/// interchangeable.
pub(crate) fn build_analysis(
    graph: &graphify_build::Graph,
    communities: &indexmap::IndexMap<i64, Vec<String>>,
    root: &std::path::Path,
    // (input, output) LLM token cost surfaced in the report (#1694). `(0, 0)`
    // for paths that ran no LLM calls.
    token_cost: (u64, u64),
) -> serde_json::Value {
    let mut communities_json = serde_json::Map::new();
    for (cid, members) in communities {
        communities_json.insert(
            cid.to_string(),
            serde_json::Value::Array(
                members
                    .iter()
                    .map(|m| serde_json::Value::String(m.clone()))
                    .collect(),
            ),
        );
    }
    let cohesion = graphify_cluster::score_all(graph, communities);
    let mut cohesion_json = serde_json::Map::new();
    for (cid, score) in &cohesion {
        cohesion_json.insert(
            cid.to_string(),
            serde_json::Number::from_f64(*score)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
        );
    }
    let god_nodes = graphify_analyze::god_nodes(graph, 12);
    let surprising = graphify_analyze::surprising_connections(graph, communities, 12);
    let empty_labels: indexmap::IndexMap<i64, String> = indexmap::IndexMap::new();
    let suggested = graphify_analyze::suggest_questions(graph, communities, &empty_labels, 8);
    serde_json::json!({
        "root": root.display().to_string(),
        "communities": serde_json::Value::Object(communities_json),
        // Python-compatible keys (read by export wiki/obsidian and Python's report).
        "cohesion": serde_json::Value::Object(cohesion_json.clone()),
        "gods": god_nodes.clone(),
        "surprises": surprising.clone(),
        "tokens": serde_json::json!({"input": token_cost.0, "output": token_cost.1}),
        // Rust report aliases (read by graphify_report::render_report).
        "cohesion_scores": serde_json::Value::Object(cohesion_json),
        "god_nodes": god_nodes,
        "surprising_connections": surprising,
        // `token_cost` is the form `graphify_report::render_report` reads (#1694).
        "token_cost": serde_json::json!({"input": token_cost.0, "output": token_cost.1}),
        "suggested_questions": suggested,
        "min_community_size": 3,
    })
}
