//! MCP stdio server — exposes graphify graph queries to LLM clients.
//!
//! Ports `graphify-py/graphify/serve.py`.
//!
//! # Modules
//!
//! - [`graph`]  — pure graph-query helpers (`_score_nodes`, `_bfs`, etc.)
//! - [`tools`]  — MCP tool handler implementations
//! - [`server`] — JSON-RPC stdio transport

pub mod graph;
pub mod server;
pub mod tools;

use thiserror::Error;

/// Errors produced by the serve layer.
#[derive(Debug, Error)]
pub enum ServeError {
    /// The graph path is invalid (wrong extension, etc.).
    #[error("error: Graph path must be a .json file, got: {0}")]
    InvalidPath(String),

    /// Graph file was not found on disk.
    #[error("error: {0}")]
    NotFound(String),

    /// The graph JSON could not be parsed.
    #[error("error: graph.json is corrupted ({0}). Re-run /graphify to rebuild.")]
    CorruptedGraph(String),

    /// Low-level I/O error.
    #[error("error: {0}")]
    Io(String),
}

/// Hot-reload state: tracks `(mtime_ns, size)` to detect file changes.
///
/// Mirrors Python `_reload_state` dict.
#[derive(Debug, Clone, Default)]
pub struct ReloadState {
    pub mtime_ns: u64,
    pub size: u64,
}

/// Start the MCP server on the real stdio streams.
///
/// # Errors
///
/// Returns [`ServeError`] if the graph file cannot be loaded.
pub async fn serve(graph_path: &str) -> Result<(), ServeError> {
    use tokio::io::{stdin, stdout};
    server::run_server(stdin(), stdout(), graph_path).await
}
