//! Default global-graph paths under `~/.graphify/`.

use std::path::PathBuf;

/// Return the default `~/.graphify` directory.
///
/// Falls back to the current directory if `HOME` cannot be resolved —
/// should not happen in normal operation but avoids a panic in degenerate
/// environments.
fn default_global_dir() -> PathBuf {
    // `std::env::home_dir` was un-deprecated in Rust 1.86 and now returns
    // the correct value on Windows, so no third-party crate is needed.
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".graphify")
}

/// Return the default path of the global graph JSON file
/// (`~/.graphify/global-graph.json`).
#[must_use]
pub fn global_graph_path() -> PathBuf {
    default_global_dir().join("global-graph.json")
}

/// Return the default path of the global manifest JSON file
/// (`~/.graphify/global-manifest.json`).
#[must_use]
pub fn global_manifest_path() -> PathBuf {
    default_global_dir().join("global-manifest.json")
}
