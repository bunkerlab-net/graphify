//! `watch` and `check-update` commands — filesystem watcher and update-flag check.

use anyhow::Result;

/// Watch a folder and rebuild the graph on code changes.
///
/// Runs `graphify_watch::watch` with a 1-second debounce until Ctrl-C.
/// Mirrors Python's `watch` command at `__main__.py`.
pub(crate) fn cmd_watch(path: &std::path::Path) -> Result<()> {
    // Match Python's `watch()` default debounce of 3.0 s. The window swallows
    // rapid bursts of editor save events so a single multi-file save does not
    // trigger N rebuilds.
    let debounce_secs = 3.0;
    eprintln!(
        "watching {} (debounce={debounce_secs}s, Ctrl-C to stop) ...",
        path.display()
    );
    graphify_watch::watch(path, debounce_secs)?;
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
