//! Error types for `graphify-detect`.

use std::path::PathBuf;

/// Errors from file discovery and manifest operations.
#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Root path does not exist: {0}")]
    RootMissing(PathBuf),

    #[error("Office conversion error: {0}")]
    Office(String),
}
