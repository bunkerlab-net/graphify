//! Error type returned by [`crate::assert_valid`].

use thiserror::Error;

/// Validation error: a non-empty list of issues that prevents the
/// extraction from being accepted.
#[derive(Debug, Error)]
#[error("Extraction JSON has {} error(s):\n{}", errors.len(), errors.iter().map(|e| format!("  • {e}")).collect::<Vec<_>>().join("\n"))]
pub struct ValidationError {
    /// The list of error messages — same content as
    /// [`crate::validate_extraction`].
    pub errors: Vec<String>,
}
