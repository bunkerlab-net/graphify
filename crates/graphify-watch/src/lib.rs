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
mod watch_fn;

pub use constants::WATCHED_EXTENSIONS;
pub use error::WatchError;
pub use lock::RebuildLock;
pub use notify::{check_update, graphify_out, notify_only};
pub use rebuild::{check_shrink, git_head, node_community_map, relativize_source_files};
pub use resource::apply_resource_limits;
pub use watch_fn::{rebuild_code, watch};
