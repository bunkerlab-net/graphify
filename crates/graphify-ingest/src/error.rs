//! Error type for ingest operations.

use std::path::PathBuf;

use thiserror::Error;

use graphify_security::SecurityError;

/// Errors produced by the ingest module.
#[derive(Debug, Error)]
pub enum IngestError {
    /// URL failed security validation.
    #[error("ingest: {0}")]
    InvalidUrl(String),

    /// Network / HTTP fetch failure.
    #[error("ingest: failed to fetch {url:?}: {source}")]
    FetchFailed {
        /// The URL that failed to fetch.
        url: String,
        /// The underlying transport / security error.
        #[source]
        source: SecurityError,
    },

    /// Filesystem I/O failure.
    #[error("ingest: I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Filename collision counter exhausted after 999 attempts.
    #[error("ingest: could not find a free filename after 999 attempts for {0:?}")]
    FilenameFull(PathBuf),
}
