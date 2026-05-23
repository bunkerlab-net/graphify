//! Error type for the export layer.

use thiserror::Error;

/// Errors produced by the export layer.
#[derive(Debug, Error)]
pub enum ExportError {
    /// Underlying filesystem I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation / deserialisation error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// The graph has more nodes than the HTML viz limit.
    ///
    /// Callers should suggest `--no-viz`, raising
    /// `GRAPHIFY_VIZ_NODE_LIMIT`, or reducing the input size.
    #[error(
        "graph too large for HTML viz ({nodes} nodes, limit {limit}). Use --no-viz, raise GRAPHIFY_VIZ_NODE_LIMIT, or reduce input size."
    )]
    TooLargeForViz {
        /// Node count of the rejected graph.
        nodes: usize,
        /// Limit that was exceeded.
        limit: usize,
    },

    /// Catch-all for export errors that don't fit the categories above.
    #[error("{0}")]
    Other(String),
}
