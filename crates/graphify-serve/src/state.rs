//! Hot-reload state for the MCP server.

/// Tracks `(mtime_ns, size)` so the server can detect when the graph
/// file on disk has changed and trigger an in-memory reload.
///
/// Mirrors the Python `_reload_state` dict.
#[derive(Debug, Clone, Default)]
pub struct ReloadState {
    /// Last-seen modified-time in nanoseconds since Unix epoch.
    pub mtime_ns: u64,
    /// Last-seen file size in bytes.
    pub size: u64,
}
