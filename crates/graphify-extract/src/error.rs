//! Error types for graphify-extract.

use thiserror::Error;

/// Errors that can occur during extraction.
#[derive(Debug, Error)]
pub enum ExtractError {
    /// The language has no tree-sitter crate available.
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(&'static str),

    /// I/O failure reading source file.
    #[error("io error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Tree-sitter parse step failed.
    #[error("parse error for {path}: {message}")]
    Parse { path: String, message: String },

    /// JSON decode/encode failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
