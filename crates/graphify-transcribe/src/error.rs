//! Error type for transcription and audio-download operations.

use std::path::PathBuf;

use thiserror::Error;

use graphify_security::SecurityError;

/// Errors produced by transcription and audio-download operations.
#[derive(Debug, Error)]
pub enum TranscribeError {
    /// A required binary (`whisper-cli` or `yt-dlp`) was not found on PATH.
    #[error("Required binary '{binary}' not found on PATH")]
    BinaryMissing {
        /// The binary name (e.g. `"whisper-cli"`).
        binary: String,
    },

    /// The binary exited with a non-zero status.
    #[error("'{binary}' failed (exit {code}): {stderr}")]
    BinaryFailed {
        /// The binary name (e.g. `"yt-dlp"`).
        binary: String,
        /// The process exit code (or `-1` if it was killed by a signal).
        code: i32,
        /// Captured stderr from the failed process.
        stderr: String,
    },

    /// URL validation failed (SSRF guard, bad scheme, etc.).
    #[error("URL validation failed: {0}")]
    InvalidUrl(#[from] SecurityError),

    /// A filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Transcript output was not found after the whisper-cli run.
    #[error("whisper-cli did not produce expected transcript at {path}")]
    OutputMissing {
        /// The path that was expected to exist.
        path: PathBuf,
    },
}
