//! Error type for the MCP serve layer.

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
