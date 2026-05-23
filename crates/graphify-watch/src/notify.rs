//! Pending-update flag file + user-facing notifications for non-code
//! changes that require LLM re-extraction.

use std::path::Path;

use crate::constants::DEFAULT_GRAPHIFY_OUT;
use crate::error::WatchError;

/// Return the effective output directory name from the environment.
///
/// Reads `GRAPHIFY_OUT`, falling back to the compile-time default
/// (`"graphify-out"`).
#[must_use]
pub fn graphify_out() -> String {
    std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| DEFAULT_GRAPHIFY_OUT.to_string())
}

/// Write a `needs_update` flag file and print a notification.
///
/// Called for non-code file changes (docs, papers, images) that require
/// LLM re-extraction via `graphify --update`.
///
/// Ports `_notify_only` from Python.
///
/// # Errors
///
/// Returns [`WatchError::Io`] if the flag file cannot be created.
pub fn notify_only(watch_path: &Path) -> Result<(), WatchError> {
    let out = watch_path.join(graphify_out());
    let flag = out.join("needs_update");
    std::fs::create_dir_all(&out).map_err(WatchError::Io)?;
    std::fs::write(&flag, "1").map_err(WatchError::Io)?;
    println!(
        "\n[graphify watch] New or changed files detected in {}",
        watch_path.display()
    );
    println!("[graphify watch] Non-code files changed - semantic re-extraction requires LLM.");
    println!("[graphify watch] Run `/graphify --update` in Claude Code to update the graph.");
    println!("[graphify watch] Flag written to {}", flag.display());
    Ok(())
}

/// Check for a pending semantic update flag and notify the user if set.
///
/// Always returns `true` so cron jobs do not alarm. Non-code file
/// changes require LLM-backed re-extraction — this function only signals
/// that the update is needed.
///
/// Ports `check_update` from Python.
#[must_use]
pub fn check_update(watch_path: &Path) -> bool {
    let flag = watch_path.join(graphify_out()).join("needs_update");
    if flag.exists() {
        println!(
            "[graphify check-update] Pending non-code changes in {}.",
            watch_path.display()
        );
        println!(
            "[graphify check-update] Run `/graphify --update` to apply semantic re-extraction."
        );
    }
    true
}
