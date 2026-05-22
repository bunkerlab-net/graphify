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
}
