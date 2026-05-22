//! `watch` and `check-update` commands — filesystem watcher and update-flag check.

use anyhow::Result;

/// Watch a folder and rebuild the graph on code changes.
///
/// Runs `graphify_watch::watch` with a 1-second debounce until Ctrl-C.
/// Mirrors Python's `watch` command at `__main__.py`.
pub(crate) fn cmd_watch(path: &std::path::Path) -> Result<()> {
    eprintln!(
        "watching {} (debounce=1s, Ctrl-C to stop) ...",
        path.display()
    );
    graphify_watch::watch(path, 1.0)?;
    Ok(())
}

/// Check the `needs_update` flag and exit 0 if set, 1 otherwise.
///
/// Used by CI/editor hooks to detect when a semantic re-extraction is pending.
/// Mirrors Python's `check-update` command at `__main__.py`.
pub(crate) fn cmd_check_update(path: &std::path::Path) -> Result<()> {
    if graphify_watch::check_update(path) {
        std::process::exit(0);
    }
    std::process::exit(1);
}
