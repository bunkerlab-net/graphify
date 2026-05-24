//! Error type produced by the wiki generator.

use thiserror::Error;

/// Errors produced by [`crate::to_wiki`].
#[derive(Debug, Error)]
pub enum WikiError {
    /// `communities` argument was empty after stale-ID filtering.
    #[error(
        "communities dict is empty — refusing to clear wiki/. \
         Run `graphify extract .` or `graphify cluster-only .` first."
    )]
    EmptyCommunities,

    /// All community node IDs were stale relative to the graph.
    #[error(
        "all community node IDs are stale — none exist in the graph. \
         Re-run `graphify extract .` to regenerate .graphify_analysis.json."
    )]
    AllStale,

    /// An underlying filesystem I/O error occurred.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
