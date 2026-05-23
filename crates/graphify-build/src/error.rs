//! Error type for the graph build layer.

use thiserror::Error;

/// Build-layer errors.
#[derive(Debug, Error)]
pub enum BuildError {
    /// `build_merge` would have removed nodes from the graph; the caller
    /// must opt in to pruning by passing `prune_sources` explicitly.
    #[error(
        "graphify: build_merge would shrink graph from {prev} → {now} nodes. Pass prune_sources explicitly if you intend to remove nodes."
    )]
    WouldShrink {
        /// Node count before the merge.
        prev: usize,
        /// Node count after the merge.
        now: usize,
    },

    /// Underlying filesystem I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON serialisation / deserialisation error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
