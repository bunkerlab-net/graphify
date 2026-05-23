//! Read the current install status of graphify's post-commit and
//! post-checkout hooks.

use std::fs;
use std::path::Path;

use crate::constants::{CHECKOUT_MARKER, HOOK_MARKER};
use crate::git::{git_root, hooks_dir};

/// Return a single-hook status string: `"installed"`, `"not installed"`, or
/// `"not installed (hook exists but graphify not found)"`.
fn check_hook(hdir: &Path, name: &str, marker: &str) -> String {
    let p = hdir.join(name);
    if !p.exists() {
        return "not installed".to_string();
    }
    match fs::read_to_string(&p) {
        Ok(content) if content.contains(marker) => "installed".to_string(),
        Ok(_) | Err(_) => "not installed (hook exists but graphify not found)".to_string(),
    }
}

/// Return a human-readable two-line status string describing whether the
/// post-commit and post-checkout hooks are installed.
///
/// Never fails — returns `"Not in a git repository."` if `path` is outside
/// any git repository or the hooks directory cannot be resolved.
#[must_use]
pub fn status(path: &Path) -> String {
    let Some(root) = git_root(path) else {
        return "Not in a git repository.".to_string();
    };

    let Ok(hdir) = hooks_dir(&root) else {
        return "Not in a git repository.".to_string();
    };

    let commit = check_hook(&hdir, "post-commit", HOOK_MARKER);
    let checkout = check_hook(&hdir, "post-checkout", CHECKOUT_MARKER);
    format!("post-commit: {commit}\npost-checkout: {checkout}")
}
