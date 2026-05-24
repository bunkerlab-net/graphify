//! Error type for the token-reduction benchmark.

use thiserror::Error;

/// Errors that can occur during benchmarking.
#[derive(Debug, Error)]
pub enum BenchmarkError {
    /// Underlying filesystem I/O error reading the graph file.
    #[error("I/O error reading graph file: {0}")]
    Io(#[from] std::io::Error),

    /// JSON parse error.
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// Error returned by the build layer.
    #[error(transparent)]
    Build(#[from] graphify_build::BuildError),

    /// Graph file exceeds the memory-bomb size cap.
    #[error(transparent)]
    Security(#[from] graphify_security::SecurityError),
}
