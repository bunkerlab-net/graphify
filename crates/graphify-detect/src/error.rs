//! Error types for `graphify-detect`.

use std::path::PathBuf;

/// Errors from file discovery and manifest operations.
#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    /// Returned when an underlying filesystem or I/O operation fails.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Returned when a manifest file contains malformed JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Returned when the scan root path does not exist on disk.
    #[error("Root path does not exist: {0}")]
    RootMissing(PathBuf),

    /// Returned when a PDF, DOCX, or XLSX conversion step fails.
    #[error("Office conversion error: {0}")]
    Office(String),
}
