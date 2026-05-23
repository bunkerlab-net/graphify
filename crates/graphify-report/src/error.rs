//! Error type for report rendering.

use thiserror::Error;

/// Errors from [`crate::write_report`].
#[derive(Debug, Error)]
pub enum ReportError {
    /// I/O error writing the report file.
    #[error("graphify: failed to write report: {0}")]
    Io(#[from] std::io::Error),
}
