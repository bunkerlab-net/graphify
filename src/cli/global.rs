//! `global` subcommands — manage the global graph (~/.graphify/global-graph.json).

use anyhow::Result;

use crate::GlobalCmd;

/// Manage the global graph (`~/.graphify/global-graph.json`).
///
/// Dispatches to `global_add`, `global_remove`, `global_list`, or `global_graph_path`
/// based on the subcommand. Mirrors the `global` elif chain in `__main__.py`.
pub(crate) fn cmd_global(cmd: GlobalCmd) -> Result<()> {
    match cmd {
        GlobalCmd::Add { graph, as_tag } => {
            // Mirror Python's `source.parent.parent.name`: for
            // `<repo>/graphify-out/graph.json` this resolves to `<repo>`.
            let tag = as_tag.unwrap_or_else(|| {
                graph
                    .parent()
                    .and_then(std::path::Path::parent)
                    .and_then(std::path::Path::file_name)
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });
            let manifest_path = graphify_global::global_manifest_path();
            let global_path = graphify_global::global_graph_path();
            let summary = graphify_global::global_add(&graph, &tag, &global_path, &manifest_path)?;
            if summary.nodes_added == 0 && summary.nodes_removed == 0 {
                println!("'{tag}' unchanged since last add — global graph not modified.");
            } else {
                println!(
                    "Added '{tag}' to global graph: +{} nodes, -{} pruned. Global: {}",
                    summary.nodes_added,
                    summary.nodes_removed,
                    global_path.display()
                );
            }
        }
        GlobalCmd::Remove { tag } => {
            let manifest_path = graphify_global::global_manifest_path();
            let global_path = graphify_global::global_graph_path();
            let removed = graphify_global::global_remove(&tag, &global_path, &manifest_path)?;
            println!("Removed '{tag}' from global graph ({removed} nodes pruned).");
        }
        GlobalCmd::List => {
            let manifest_path = graphify_global::global_manifest_path();
            let entries = graphify_global::global_list(&manifest_path);
            for (tag, entry) in &entries {
                println!(
                    "{tag}\t{} nodes\t{} edges\t{}",
                    entry.node_count, entry.edge_count, entry.added_at
                );
            }
        }
        GlobalCmd::Path => {
            println!("{}", graphify_global::global_graph_path().display());
        }
    }
    Ok(())
}
