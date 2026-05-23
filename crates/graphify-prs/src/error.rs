//! Error type for the `graphify-prs` crate.

/// Errors that can occur during PR operations.
#[derive(Debug, thiserror::Error)]
pub enum PrsError {
    #[error("gh CLI not found or not authenticated: {0}\nRun: gh auth login")]
    GhNotFound(String),

    #[error("gh CLI returned an error: {0}")]
    GhFailed(String),

    #[error("Failed to parse PR JSON: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("Failed to parse PR updated_at date: {0}")]
    DateParse(String),

    #[error("PR #{0} not found in open PRs")]
    PrNotFound(u64),
}
