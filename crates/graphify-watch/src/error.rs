//! Error type for `graphify-watch`.

/// Errors that can occur in the watch module.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// An I/O error (flag file creation, lock file, etc.).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A `notify` watcher error.
    #[error("watcher error: {0}")]
    Notify(#[from] notify::Error),

    /// A pipeline-stage error (build, cluster, report, export, etc.).
    #[error("pipeline error: {0}")]
    Pipeline(String),

    /// Shrink guard refused to overwrite — new graph has fewer nodes than existing.
    #[error(
        "graphify: new graph has {new} nodes but existing graph.json has {existing}; \
         refusing to overwrite (pass --force to override)"
    )]
    ShrinkRefused {
        /// Node count in the existing graph.
        existing: usize,
        /// Node count in the candidate graph.
        new: usize,
    },
}
