//! Error type for the deduplication pipeline.

use thiserror::Error;

/// Errors produced by the deduplication pipeline.
#[derive(Debug, Error)]
pub enum DedupError {
    /// Nodes span more than one repository; cross-project dedup is
    /// disabled.
    #[error(
        "deduplicate_entities: nodes span multiple repos {0}. \
         Cross-project dedup is disabled — run dedup per-repo before merging."
    )]
    MultipleRepos(String),

    /// `pick_winner` was called with an empty candidate list.
    #[error("Cannot pick winner from empty list")]
    EmptyGroup,
}
