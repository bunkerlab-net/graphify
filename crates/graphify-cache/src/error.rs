//! Error type for the per-file extraction cache.

use std::path::PathBuf;

/// Cache-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// `file_hash` was called with a path that is not a regular file.
    #[error("file_hash requires a file, got: {0}")]
    NotAFile(PathBuf),

    /// Underlying filesystem I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON serialisation / deserialisation error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
