//! `serve` command — MCP stdio server.

use anyhow::Result;

use crate::cli::graphify_out_dir;

/// Start the MCP stdio server backed by `graph.json`.
///
/// Builds a single-threaded Tokio runtime and blocks on
/// `graphify_serve::serve`. Mirrors Python's `serve` command at `__main__.py`.
pub(crate) fn cmd_serve(graph: Option<&std::path::Path>) -> Result<()> {
    let default_path = graphify_out_dir().join("graph.json");
    let path = graph.unwrap_or(default_path.as_path());
    eprintln!(
        "serving MCP over stdio (graph={}, Ctrl-C to stop) ...",
        path.display()
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(graphify_serve::serve(&path.to_string_lossy()))?;
    Ok(())
}
