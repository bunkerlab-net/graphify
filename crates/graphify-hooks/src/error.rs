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

    /// A configured git hooks path looks like a Windows absolute or backslash
    /// path, which cannot resolve to a real directory on WSL/POSIX (#1385).
    /// Carries where the value came from and the offending value.
    #[error(
        "git hooks path from {origin} looks like a Windows path: {value:?}. \
         On WSL/POSIX this can't resolve to a real directory. Unset it with \
         `git config --local --unset core.hooksPath`, or set a POSIX path."
    )]
    WindowsPath {
        /// Where the value came from: `core.hooksPath` or
        /// `git rev-parse --git-path hooks`.
        origin: &'static str,
        /// The offending raw value.
        value: String,
    },
}
