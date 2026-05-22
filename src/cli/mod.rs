//! CLI module declarations and shared utilities.
//!
//! Each submodule owns the handler function(s) for the corresponding command
//! group.  Shared helpers used by more than one command live here.

pub(crate) mod add;
pub(crate) mod benchmark;
pub(crate) mod cache_check;
pub(crate) mod clone;
pub(crate) mod cluster_only;
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
/// command that needs to traverse or query the graph.
pub(crate) fn load_graph(path: &std::path::Path) -> anyhow::Result<graphify_build::Graph> {
    let contents = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    let graph = graphify_build::build_from_json(value, true, None)?;
    Ok(graph)
}

/// Build the analysis JSON consumed by `graphify_report::write_report`.
///
/// Mirrors the shape produced by Python's `analyze.generate(...)` for the
/// minimum set of fields the report renderer reads.
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
    let god_nodes = graphify_analyze::god_nodes(graph, 12);
    let surprising = graphify_analyze::surprising_connections(graph, communities, 12);
    let empty_labels: indexmap::IndexMap<i64, String> = indexmap::IndexMap::new();
    let suggested = graphify_analyze::suggest_questions(graph, communities, &empty_labels, 8);
    serde_json::json!({
        "root": root.display().to_string(),
        "communities": serde_json::Value::Object(communities_json),
        "god_nodes": god_nodes,
        "surprising_connections": surprising,
        "suggested_questions": suggested,
        "min_community_size": 3,
    })
}
