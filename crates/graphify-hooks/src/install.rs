//! Install graphify's post-commit and post-checkout hooks.

use std::fs;
use std::path::Path;

use crate::constants::{CHECKOUT_MARKER, CHECKOUT_SCRIPT, HOOK_MARKER, HOOK_SCRIPT};
use crate::error::HooksError;
use crate::git::{git_root, hooks_dir, user_hooks_dir};

/// Install a single git hook file.
///
/// If the hook file already exists and contains `marker`, this is a no-op.
/// Otherwise the script is either appended to an existing hook (preserving
/// other content) or used to create a new hook with a `#!/bin/sh` shebang
/// and executable bits.
///
/// Returns a human-readable result message describing what was done.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub(crate) fn install_hook(
    hooks_dir: &Path,
    name: &str,
    script: &str,
    marker: &str,
) -> Result<String, HooksError> {
    let hook_path = hooks_dir.join(name);
    if hook_path.exists() {
        let content = fs::read_to_string(&hook_path)?;
        if content.contains(marker) {
            return Ok(format!("already installed at {}", hook_path.display()));
        }
        let new_content = content.trim_end().to_string() + "\n\n" + script;
        fs::write(&hook_path, new_content.as_bytes())?;
        // Re-assert executable bits in case the existing file was created
        // by hand without `chmod +x`.
        set_executable(&hook_path)?;
        return Ok(format!(
            "appended to existing {name} hook at {}",
            hook_path.display()
        ));
    }
    let full = format!("#!/bin/sh\n{script}");
    fs::write(&hook_path, full.as_bytes())?;
    set_executable(&hook_path)?;
    Ok(format!("installed at {}", hook_path.display()))
}

/// Set the executable bits (`o+x`, `g+x`, `u+x`) on `path`.
///
/// No-op on non-Unix platforms.
///
/// # Errors
///
/// Returns `HooksError::Io` if the file's metadata or permissions cannot be
/// read or updated.
pub(crate) fn set_executable(path: &Path) -> Result<(), HooksError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        let mode = perms.mode();
        perms.set_mode(mode | 0o111);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Install graphify's post-commit and post-checkout hooks into the nearest
/// git repository at or above `path`.
///
/// Returns a multi-line status message reporting the outcome for each hook.
///
/// # Errors
///
/// - `HooksError::NotAGitRepo` if no git repository is found at or above
///   `path`.
/// - `HooksError::Io` on filesystem failures.
pub fn install(path: &Path) -> Result<String, HooksError> {
    let root = git_root(path).ok_or_else(|| {
        let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        HooksError::NotAGitRepo(resolved)
    })?;

    let hdir = user_hooks_dir(&hooks_dir(&root)?);

    // Substitute the `__PINNED_PYTHON__` placeholder. The Python install pins
    // `sys.executable` here so the hook works without the launcher on PATH; the
    // Rust binary is not a Python interpreter, so there is nothing to pin —
    // the empty string makes the hook skip the pinned probe and fall through to
    // its runtime detection (`.graphify_python`, launcher shebang, python3).
    let pinned = pinned_python();
    let hook = HOOK_SCRIPT.replace("__PINNED_PYTHON__", &pinned);
    let checkout = CHECKOUT_SCRIPT.replace("__PINNED_PYTHON__", &pinned);

    let commit_msg = install_hook(&hdir, "post-commit", &hook, HOOK_MARKER)?;
    let checkout_msg = install_hook(&hdir, "post-checkout", &checkout, CHECKOUT_MARKER)?;

    Ok(format!(
        "post-commit: {commit_msg}\npost-checkout: {checkout_msg}"
    ))
}

/// The interpreter path embedded into the hook's pinned probe.
///
/// The Rust binary has no `sys.executable` to pin (it is not a Python
/// interpreter), so this is the empty string and the hook relies on its
/// runtime detection chain. Routed through the same allowlist the hook applies
/// so a future real pin degrades safely.
fn pinned_python() -> String {
    let candidate = String::new();
    if candidate
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && !"/_.@:\\-".contains(c))
    {
        return String::new();
    }
    candidate
}
