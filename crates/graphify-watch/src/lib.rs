//! Filesystem watcher that rebuilds the graph on file changes.
//!
//! Ports `graphify-py/graphify/watch.py`.
//!
//! # Architecture
//!
//! The primary entry point for production use is [`watch`], which
//! spawns a `notify_debouncer_full` watcher and batches events over a
//! configurable debounce window. The full rebuild pipeline lives in
//! [`rebuild`].

pub mod canonical;
mod constants;
pub mod error;
pub mod lock;
mod notify;
pub mod rebuild;
mod resource;
pub mod test_support;
mod watch_fn;

pub use constants::WATCHED_EXTENSIONS;
pub use error::WatchError;
pub use lock::RebuildLock;
pub use notify::{check_update, notify_only};
pub use rebuild::{
    LockPolicy, PENDING_DRAIN_MAX_PASSES, PENDING_FILENAME, RebuildOptions, check_shrink,
    drain_pending, git_head, merge_changed_paths, node_community_map, queue_pending,
    rebuild_with_pending, relativize_source_files,
};
pub use resource::apply_resource_limits;
pub use watch_fn::{rebuild_code, watch};
