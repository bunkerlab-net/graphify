//! Error type shared by all hook and platform-install operations.

use std::path::PathBuf;

use thiserror::Error;

/// Errors from hook installation/uninstallation.
#[derive(Debug, Error)]
pub enum HooksError {
    /// No git repository found at or above the given path.
    #[error("No git repository found at or above {0}")]
    NotAGitRepo(PathBuf),

    /// Filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation/deserialisation error.
    #[error("JSON error: {0}")]
    Json(String),

    /// Unknown platform name passed to `install_platform_skill`.
    #[error("unknown platform '{0}'")]
    UnknownPlatform(String),
}
