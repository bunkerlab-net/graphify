//! `export` subcommands — export graph to various formats (`HTML`, `SVG`, `GraphML`,
//! `Obsidian`, `Wiki`, Mermaid call-flow HTML, `Neo4j`).

use anyhow::Result;

use crate::ExportCmd;
use crate::cli::{default_graph_path, graphify_out_dir, load_graph};

#[allow(clippy::too_many_lines)]
// reason: dispatch over every ExportCmd variant; splitting adds indirection
// without reducing complexity; mirrors Python's elif chain in __main__.py.
/// Dispatch the `export` subcommand to the appropriate format renderer.
///
/// Each arm loads `graph.json`, clusters if needed, and writes the output
/// file. Mirrors the `export` elif chain in `__main__.py`.
pub(crate) fn cmd_export(cmd: ExportCmd) -> Result<()> {
    match cmd {
        ExportCmd::Graphml { graph } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out = path.with_file_name("graph.graphml");
            eprintln!(
                "exporting GraphML ({} nodes, {} edges) ...",
                g.node_count(),
                g.edge_count()
            );
            graphify_export::to_graphml(&g, &communities, &out)?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Svg { graph } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out = path.with_file_name("graph.svg");
            eprintln!(
                "computing spring layout + rendering SVG for {} nodes ...",
                g.node_count()
            );
            graphify_export::to_svg(&g, &communities, &out, None, (16, 12))?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Html { graph } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out = path.with_file_name("graph.html");
            eprintln!("rendering HTML viz ({} nodes) ...", g.node_count());
            graphify_export::to_html(&g, &communities, &out, None, None, None)?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Obsidian { graph, out } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let communities = indexmap::IndexMap::new();
            let out_dir = out.unwrap_or_else(|| graphify_out_dir().join("obsidian"));
            eprintln!(
                "rendering Obsidian vault ({} nodes) to {} ...",
                g.node_count(),
                out_dir.display()
            );
            let count = graphify_export::to_obsidian(&g, &communities, &out_dir, None, None)?;
            eprintln!("wrote {count} notes to {}", out_dir.display());
        }
        ExportCmd::Wiki { graph } => {
            // Load graph and cluster to produce communities, then call to_wiki.
            // Mirrors Python's export wiki path at __main__.py:2283.
            let path = graph.unwrap_or_else(default_graph_path);
            let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            // Re-cluster inline because we need community data for the wiki.
            eprintln!(
                "clustering {} nodes for wiki export (resolution=1.0) ...",
                g.node_count()
            );
            let communities = graphify_cluster::cluster(&g, 1.0, None);
            if communities.is_empty() {
                anyhow::bail!(
                    "graph produced no communities — run `graphify extract .` (or \
                     `graphify cluster-only .`) to regenerate community data first"
                );
            }
            let wiki_dir = out_dir.join("wiki");
            eprintln!(
                "writing wiki ({} communities) to {} ...",
                communities.len(),
                wiki_dir.display()
            );
            let n = graphify_wiki::to_wiki(&g, &communities, &wiki_dir, None, None, None)?;
            println!("Wiki: {n} articles written to {}", wiki_dir.display());
            println!("  {}/index.md  ->  agent entry point", wiki_dir.display());
        }
        ExportCmd::CallflowHtml {
            graph,
            output,
            lang,
            max_sections,
            diagram_scale,
            max_diagram_nodes,
            max_diagram_edges,
            report,
            sections,
        } => {
            eprintln!("rendering Mermaid call-flow HTML ...");
            let mut opts = graphify_html::callflow::CallflowOptions {
                graph,
                output,
                report,
                sections,
                ..Default::default()
            };
            // Apply flag overrides only when the user explicitly provided them.
            if let Some(l) = lang {
                opts.lang = l;
            }
            if let Some(ms) = max_sections {
                opts.max_sections = ms;
            }
            if let Some(ds) = diagram_scale {
                opts.diagram_scale = ds;
            }
            if let Some(mdn) = max_diagram_nodes {
                opts.max_diagram_nodes = mdn;
            }
            if let Some(mde) = max_diagram_edges {
                opts.max_diagram_edges = mde;
            }
            let written = graphify_html::callflow::write_callflow_html(&opts)?;
            eprintln!("wrote {}", written.display());
        }
        ExportCmd::Neo4j { graph: _ } => {
            anyhow::bail!("export neo4j: requires neo4rs integration (deferred)")
        }
    }
    Ok(())
}
