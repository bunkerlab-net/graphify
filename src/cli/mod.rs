//! CLI module declarations and shared utilities.
//!
//! Each submodule owns the handler function(s) for the corresponding command
//! group.  Shared helpers used by more than one command live here.

pub(crate) mod add;
pub(crate) mod args;
pub(crate) mod benchmark;
pub(crate) mod cache_check;
pub(crate) mod clone;
pub(crate) mod cluster_only;
pub(crate) mod dispatch;
pub(crate) mod export;
pub(crate) mod extract;
pub(crate) mod global;
pub(crate) mod hooks;
pub(crate) mod install;
pub(crate) mod merge;
pub(crate) mod merge_chunks;
pub(crate) mod prs;
pub(crate) mod query;
pub(crate) mod save_result;
pub(crate) mod serve;
pub(crate) mod tree;
pub(crate) mod validate;
pub(crate) mod watch;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

/// Configure runtime services, parse argv, and dispatch the selected subcommand.
///
/// Registers the `graphify-cache` atexit flush, initialises `tracing`, then
/// parses [`args::Cli`] and forwards to [`dispatch::dispatch`]. When no
/// subcommand is supplied, prints a help hint and returns `Ok(())`.
pub(crate) fn run() -> Result<()> {
    graphify_cache::ensure_atexit_flush_registered();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let parsed = args::Cli::parse();
    match parsed.command {
        None => {
            println!("graphify {} — run with --help", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(cmd) => dispatch::dispatch(cmd),
    }
}

/// Return the graphify output directory, honouring the `GRAPHIFY_OUT` env var.
///
/// Python equivalent: `os.environ.get("GRAPHIFY_OUT", "graphify-out")` at
/// `__main__.py:19`. Accepts a relative name ("graphify-out-feature") or an
/// absolute path ("/shared/graphify-out").
pub(crate) fn graphify_out_dir() -> PathBuf {
    PathBuf::from(std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned()))
}

/// Return the default graph.json path, honouring `GRAPHIFY_OUT`.
pub(crate) fn default_graph_path() -> PathBuf {
    graphify_out_dir().join("graph.json")
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
    let graph = graphify_build::build_from_json(value, true, None)?;
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
        "tokens": serde_json::json!({"input": 0u64, "output": 0u64}),
        // Rust report aliases (read by graphify_report::render_report).
        "cohesion_scores": serde_json::Value::Object(cohesion_json),
        "god_nodes": god_nodes,
        "surprising_connections": surprising,
        "suggested_questions": suggested,
        "min_community_size": 3,
    })
}
