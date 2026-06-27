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

    /// Audio download failed during `YouTube` ingestion (`yt-dlp` shell-out).
    #[error("ingest: failed to fetch {url:?}: {source}")]
    Transcribe {
        /// The URL that failed to fetch.
        url: String,
        /// The underlying transcription / audio-download error.
        #[source]
        source: graphify_transcribe::TranscribeError,
    },

    /// Filesystem I/O failure.
    #[error("ingest: I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Filename collision counter exhausted after 1000 attempts (original
    /// filename plus `_1` through `_999`).
    #[error("ingest: could not find a free filename after 1000 attempts for {0:?}")]
    FilenameFull(PathBuf),

    /// `save_query_result` was given an `outcome` outside the allowed set.
    #[error(
        "ingest: outcome must be one of [\"useful\", \"dead_end\", \"corrected\"], got {got:?}"
    )]
    InvalidOutcome {
        /// The rejected outcome value.
        got: String,
    },
}
