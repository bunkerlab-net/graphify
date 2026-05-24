//! Error type for the `graphify-prs` crate.

/// Errors that can occur during PR operations.
#[derive(Debug, thiserror::Error)]
pub enum PrsError {
    /// Returned when the `gh` binary is not on `PATH` or the user is not
    /// authenticated (`gh auth login` has not been run).
    #[error("gh CLI not found or not authenticated: {0}\nRun: gh auth login")]
    GhNotFound(String),

    /// Returned when `gh` exits with a non-zero status code.
    #[error("gh CLI returned an error: {0}")]
    GhFailed(String),

    /// Returned when the JSON returned by `gh` cannot be deserialized.
    #[error("Failed to parse PR JSON: {0}")]
    JsonParse(#[from] serde_json::Error),

    /// Returned when the `updated_at` timestamp in a PR record is not a
    /// valid RFC 3339 date-time string.
    #[error("Failed to parse PR updated_at date: {0}")]
    DateParse(String),

    /// Returned by [`crate::run_cmd_prs`] when the requested PR number does
    /// not appear in the list of open PRs.
    #[error("PR #{0} not found in open PRs")]
    PrNotFound(u64),
}
