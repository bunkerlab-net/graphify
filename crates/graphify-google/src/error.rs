//! Error type for Google Workspace shortcut operations.

use std::path::PathBuf;

use thiserror::Error;

/// Errors produced by Google Workspace shortcut operations.
#[derive(Debug, Error)]
pub enum GoogleError {
    /// The shortcut file could not be read or parsed.
    #[error("could not read Google Workspace shortcut {path}: {source}")]
    ReadShortcut {
        /// The path of the file that failed to load.
        path: PathBuf,
        /// The underlying I/O or JSON parse error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The shortcut file does not contain a Drive file ID.
    #[error("Google Workspace shortcut {path} does not include a Drive file ID")]
    MissingFileId {
        /// The path of the shortcut.
        path: PathBuf,
    },

    /// The `gws` binary is missing from `PATH`.
    #[error(
        "gws is required for Google Workspace export. Install it from \
        https://github.com/googleworkspace/cli and run `gws auth login -s drive`."
    )]
    GwsMissing,

    /// `gws export` exited with a non-zero return code.
    #[error("gws export failed for {file_id}: {stderr}")]
    GwsFailed {
        /// Drive file ID that was being exported.
        file_id: String,
        /// Truncated stderr from the failed `gws` process.
        stderr: String,
    },

    /// A filesystem I/O error occurred.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The `xlsx_to_markdown` callback is required for `.gsheet` but was not
    /// provided.
    #[error(
        "Google Sheets export requires the office extra: \
        pip install graphifyy[office,google]"
    )]
    XlsxCallbackMissing,

    /// The `xlsx_to_markdown` callback failed when converting a `.gsheet`
    /// shortcut. Carries both the original shortcut path and the temp
    /// `.xlsx` file we tried to convert.
    #[error("xlsx-to-markdown conversion failed for {shortcut} (via {tmp}): {source}")]
    XlsxConversion {
        /// Path of the original `.gsheet` shortcut.
        shortcut: PathBuf,
        /// Path of the temporary `.xlsx` file we exported and tried to read.
        tmp: PathBuf,
        /// The underlying conversion error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
