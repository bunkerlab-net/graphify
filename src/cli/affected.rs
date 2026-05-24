//! `affected` command — reverse-traversal impact analysis on graph.json.

use std::path::Path;

use anyhow::Result;

use crate::cli::default_graph_path;

/// Run `graphify affected <query>`, printing the formatted report to stdout.
pub(crate) fn cmd_affected(
    query: &str,
    relations: &[String],
    depth: usize,
    graph: Option<&Path>,
) -> Result<()> {
    let graph_path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    let graph = graphify_affected::load_graph(&graph_path)?;
    let relations_refs: Vec<&str> = if relations.is_empty() {
        graphify_affected::DEFAULT_AFFECTED_RELATIONS.to_vec()
    } else {
        relations.iter().map(String::as_str).collect()
    };
    let report = graphify_affected::format_affected(&graph, query, &relations_refs, depth);
    println!("{report}");
    Ok(())
}
