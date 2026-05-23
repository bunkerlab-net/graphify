//! Error type for global-graph operations.

use std::path::PathBuf;

use thiserror::Error;

/// Errors produced by `graphify-global` operations.
#[derive(Debug, Error)]
pub enum GlobalError {
    /// The source graph file does not exist.
    #[error("graph not found: {0}")]
    GraphNotFound(PathBuf),

    /// `global_remove` was called for a repo tag not present in the manifest.
    #[error("repo '{0}' not in global graph")]
    UnknownRepo(String),

    /// An error from the build layer (graph construction).
    #[error("build error: {0}")]
    Build(String),

    /// Underlying filesystem I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON serialisation / deserialisation error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
