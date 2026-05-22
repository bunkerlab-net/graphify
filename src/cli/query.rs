//! `query`, `path`, and `explain` commands — graph traversal and inspection.

use anyhow::Result;

use crate::cli::{default_graph_path, load_graph};

/// BFS/DFS traversal of `graph.json` to find context relevant to `question`.
///
/// Delegates to `graphify_serve::graph::query_graph_text` and prints the
/// plain-text result. Mirrors Python `__main__.py`'s `query` command.
pub(crate) fn cmd_query(
    question: &str,
    dfs: bool,
    context: &[String],
    budget: usize,
    graph: Option<&std::path::Path>,
) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    eprintln!(
        "querying {} ({} nodes, mode={}) ...",
        path.display(),
        g.node_count(),
        if dfs { "dfs" } else { "bfs" }
    );
    let mode = if dfs { "dfs" } else { "bfs" };
    let context_filters: Option<&[String]> = if context.is_empty() {
        None
    } else {
        Some(context)
    };
    let mut idf_cache = std::collections::HashMap::new();
    let result = graphify_serve::graph::query_graph_text(
        &g,
        question,
        mode,
        2,
        budget,
        context_filters,
        &mut idf_cache,
    );
    println!("{result}");
    Ok(())
}

/// Print the shortest path between two nodes in `graph.json`.
///
/// Uses `graphify_serve::graph::shortest_path` (BFS). Mirrors Python's
/// `path` command at `__main__.py`.
pub(crate) fn cmd_path(from: &str, to: &str, graph: Option<&std::path::Path>) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    let g = load_graph(&path)?;
    let from_ids = graphify_serve::graph::find_node(&g, from);
    let to_ids = graphify_serve::graph::find_node(&g, to);
    let Some(src) = from_ids.first() else {
        anyhow::bail!("source node not found: {from}");
    };
    let Some(tgt) = to_ids.first() else {
        anyhow::bail!("target node not found: {to}");
    };
    match graphify_serve::graph::shortest_path(&g, src, tgt) {
        Some(p) => println!("{}", p.join(" -> ")),
        None => println!("no path from {from} to {to}"),
    }
    Ok(())
}

/// Print a node's ID and its immediate neighbors from `graph.json`.
///
/// Provides a quick plain-language summary of a single node and its connections.
/// Mirrors Python's `explain` command at `__main__.py`.
pub(crate) fn cmd_explain(node: &str, graph: Option<&std::path::Path>) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    let g = load_graph(&path)?;
    let ids = graphify_serve::graph::find_node(&g, node);
    let Some(node_id) = ids.first() else {
        anyhow::bail!("node not found: {node}");
    };
    let neighbors = graphify_serve::graph::neighbors(&g, node_id);
    println!("{node_id}");
    for n in neighbors {
        println!("  - {n}");
    }
    Ok(())
}
