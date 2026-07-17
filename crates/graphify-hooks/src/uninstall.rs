//! Remove graphify's post-commit and post-checkout hooks.

use std::fs;
use std::path::Path;

use regex::Regex;

use crate::constants::{CHECKOUT_MARKER, CHECKOUT_MARKER_END, HOOK_MARKER, HOOK_MARKER_END};
use crate::error::HooksError;
use crate::git::{git_root, hooks_dir, user_hooks_dir};

/// Remove the graphify section from a single git hook file.
///
/// Deletes the hook file outright if removing the graphify section would
/// leave only a shebang (or nothing). Otherwise writes the remaining content
/// back, preserving any non-graphify hook content.
///
/// Returns a human-readable status message.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub(crate) fn uninstall_hook(
    hooks_dir: &Path,
    name: &str,
    marker: &str,
    marker_end: &str,
) -> Result<String, HooksError> {
    let hook_path = hooks_dir.join(name);
    if !hook_path.exists() {
        return Ok(format!("no {name} hook found - nothing to remove."));
    }
    let content = fs::read_to_string(&hook_path)?;
    if !content.contains(marker) {
        return Ok(format!(
            "graphify hook not found in {name} - nothing to remove."
        ));
    }

    #[allow(clippy::expect_used)]
    let re = Regex::new(&format!(
        "(?s){}.*?{}\n?",
        regex::escape(marker),
        regex::escape(marker_end)
    ))
    .expect("pattern is constructed from escaped literals");

    let new_content = re.replace_all(&content, "").trim().to_string();
    if new_content.is_empty() || new_content == "#!/bin/bash" || new_content == "#!/bin/sh" {
        fs::remove_file(&hook_path)?;
        return Ok(format!("removed {name} hook at {}", hook_path.display()));
    }
    let final_content = new_content + "\n";
    fs::write(&hook_path, final_content.as_bytes())?;
    Ok(format!(
        "graphify removed from {name} at {} (other hook content preserved)",
        hook_path.display()
    ))
}

/// Remove graphify's post-commit and post-checkout hooks from the nearest
/// git repository at or above `path`.
///
/// Returns a multi-line status message reporting the outcome for each hook.
///
/// # Errors
///
/// - `HooksError::NotAGitRepo` if no git repository is found at or above
///   `path`.
/// - `HooksError::Io` on filesystem failures.
pub fn uninstall(path: &Path) -> Result<String, HooksError> {
    let root = git_root(path).ok_or_else(|| {
        let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        HooksError::NotAGitRepo(resolved)
    })?;

    let hdir = user_hooks_dir(&hooks_dir(&root)?);

    let commit_msg = uninstall_hook(&hdir, "post-commit", HOOK_MARKER, HOOK_MARKER_END)?;
    let checkout_msg =
        uninstall_hook(&hdir, "post-checkout", CHECKOUT_MARKER, CHECKOUT_MARKER_END)?;
    let merge_msg = crate::merge_driver::unregister_merge_driver(&root);

    Ok(format!(
        "post-commit: {commit_msg}\npost-checkout: {checkout_msg}\nmerge driver: {merge_msg}"
    ))
}
