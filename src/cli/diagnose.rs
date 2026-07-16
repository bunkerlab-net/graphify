//! `diagnose` command — read-only graph diagnostics.

use std::path::Path;

use anyhow::Result;

use crate::cli::args::DiagnoseCmd;
use crate::cli::default_graph_path;

/// Dispatch `graphify diagnose <subcommand>`.
pub(crate) fn cmd_diagnose(cmd: DiagnoseCmd) -> Result<()> {
    match cmd {
        DiagnoseCmd::Multigraph {
            graph,
            json,
            max_examples,
            directed,
            undirected,
            extract_path,
        } => cmd_diagnose_multigraph(
            graph.as_deref(),
            json,
            max_examples,
            directed,
            undirected,
            extract_path.as_deref(),
        ),
    }
}

fn cmd_diagnose_multigraph(
    graph: Option<&Path>,
    as_json: bool,
    max_examples: usize,
    directed: bool,
    undirected: bool,
    extract_path: Option<&Path>,
) -> Result<()> {
    let graph_path = graph.map_or_else(default_graph_path, Path::to_path_buf);
    let directed_override = if directed {
        Some(true)
    } else if undirected {
        Some(false)
    } else {
        None
    };
    let summary = graphify_diagnostics::diagnose_file(
        &graph_path,
        directed_override,
        max_examples,
        extract_path,
    )?;
    if as_json {
        let envelope = graphify_diagnostics::format_diagnostic_json(&summary);
        outln!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        outln!(
            "{}",
            graphify_diagnostics::format_diagnostic_report(&summary)
        );
    }
    Ok(())
}
