//! Output-directory resolution — the single source of truth for the
//! `GRAPHIFY_OUT` override across the workspace.
//!
//! The output directory is `graphify-out` by default and overridable with the
//! `GRAPHIFY_OUT` environment variable (worktrees or shared-output setups). It
//! accepts a relative name (`graphify-out-feature`) or an absolute path
//! (`/shared/graphify-out`).
//!
//! Ports `graphify-py/graphify/paths.py`. Unlike Python — which snapshots the
//! value once at import time — these read the environment on every call, so a
//! test (or a process whose environment changed) always observes the current
//! value.

use std::path::PathBuf;

/// Output directory name used when `GRAPHIFY_OUT` is unset.
pub const DEFAULT_GRAPHIFY_OUT: &str = "graphify-out";

/// The configured graphify output directory, honouring `GRAPHIFY_OUT`.
///
/// Returns a relative name (`graphify-out`) or an absolute path verbatim; the
/// caller joins it against a project root (`root.join(graphify_out())` resolves
/// correctly for both, since joining an absolute path replaces the base).
#[must_use]
pub fn graphify_out() -> PathBuf {
    PathBuf::from(std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| DEFAULT_GRAPHIFY_OUT.to_owned()))
}

/// Bare directory name even when `GRAPHIFY_OUT` is an absolute path.
///
/// Used by the path guards that walk parents looking for the output dir by
/// name, and by the detect scan-exclude so a custom output dir is never
/// re-ingested as source. Mirrors Python's
/// `os.path.basename(os.path.normpath(GRAPHIFY_OUT))`.
#[must_use]
pub fn graphify_out_name() -> String {
    graphify_out().file_name().map_or_else(
        || DEFAULT_GRAPHIFY_OUT.to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Default `graph.json` path under the configured output dir.
///
/// The package-wide fallback so a `GRAPHIFY_OUT` override is honoured wherever
/// a graph path is not passed explicitly.
#[must_use]
pub fn default_graph_json() -> PathBuf {
    graphify_out().join("graph.json")
}
