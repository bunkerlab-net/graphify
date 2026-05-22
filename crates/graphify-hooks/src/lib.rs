//! Git hook integration — install/uninstall/status graphify post-commit and
//! post-checkout hooks, plus per-platform install/uninstall functions.
//!
//! Ports `graphify-py/graphify/hooks.py` and the platform install functions
//! from `graphify-py/graphify/__main__.py`.

pub mod platform;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Public constants (tests assert on these)
// ---------------------------------------------------------------------------

/// Start marker for the post-commit hook section.
pub const HOOK_MARKER: &str = "# graphify-hook-start";
/// End marker for the post-commit hook section.
pub const HOOK_MARKER_END: &str = "# graphify-hook-end";
/// Start marker for the post-checkout hook section.
pub const CHECKOUT_MARKER: &str = "# graphify-checkout-hook-start";
/// End marker for the post-checkout hook section.
pub const CHECKOUT_MARKER_END: &str = "# graphify-checkout-hook-end";

/// Shell snippet that detects the correct Python interpreter.
/// Kept byte-identical to the Python `_PYTHON_DETECT` constant so installed
/// hooks are identical to those installed by the Python implementation.
pub const PYTHON_DETECT: &str = "\
# Detect the correct Python interpreter (handles pipx, venv, system installs)
GRAPHIFY_BIN=$(command -v graphify 2>/dev/null)
if [ -n \"$GRAPHIFY_BIN\" ]; then
    case \"$GRAPHIFY_BIN\" in
        *.exe) _SHEBANG=\"\" ;;
        *)     _SHEBANG=$(head -1 \"$GRAPHIFY_BIN\" | sed 's/^#![[:space:]]*//') ;;
    esac
    case \"$_SHEBANG\" in
        */env\\ *) GRAPHIFY_PYTHON=\"${_SHEBANG#*/env }\" ;;
        *)         GRAPHIFY_PYTHON=\"$_SHEBANG\" ;;
    esac
    # Allowlist: only keep characters valid in a filesystem path to prevent
    # injection if the shebang contains shell metacharacters
    case \"$GRAPHIFY_PYTHON\" in
        *[!a-zA-Z0-9/_.@-]*) GRAPHIFY_PYTHON=\"\" ;;
    esac
    if [ -n \"$GRAPHIFY_PYTHON\" ] && ! \"$GRAPHIFY_PYTHON\" -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"\"
    fi
fi
# Fall back: try python3, then python (Windows has no python3 shim)
if [ -z \"$GRAPHIFY_PYTHON\" ]; then
    if command -v python3 >/dev/null 2>&1 && python3 -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python3\"
    elif command -v python >/dev/null 2>&1 && python -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python\"
    else
        exit 0
    fi
fi
";

/// The full post-commit hook script, byte-identical to Python's `_HOOK_SCRIPT`.
pub const HOOK_SCRIPT: &str = "# graphify-hook-start
# Auto-rebuilds the knowledge graph after each commit (code files only, no LLM needed).
# Installed by: graphify hook install

# Skip during rebase/merge/cherry-pick to avoid blocking --continue with unstaged changes
GIT_DIR=$(git rev-parse --git-dir 2>/dev/null)
[ -d \"$GIT_DIR/rebase-merge\" ] && exit 0
[ -d \"$GIT_DIR/rebase-apply\" ] && exit 0
[ -f \"$GIT_DIR/MERGE_HEAD\" ] && exit 0
[ -f \"$GIT_DIR/CHERRY_PICK_HEAD\" ] && exit 0

CHANGED=$(git diff --name-only HEAD~1 HEAD 2>/dev/null || git diff --name-only HEAD 2>/dev/null)
if [ -z \"$CHANGED\" ]; then
    exit 0
fi

# Detect the correct Python interpreter (handles pipx, venv, system installs)
GRAPHIFY_BIN=$(command -v graphify 2>/dev/null)
if [ -n \"$GRAPHIFY_BIN\" ]; then
    case \"$GRAPHIFY_BIN\" in
        *.exe) _SHEBANG=\"\" ;;
        *)     _SHEBANG=$(head -1 \"$GRAPHIFY_BIN\" | sed 's/^#![[:space:]]*//') ;;
    esac
    case \"$_SHEBANG\" in
        */env\\ *) GRAPHIFY_PYTHON=\"${_SHEBANG#*/env }\" ;;
        *)         GRAPHIFY_PYTHON=\"$_SHEBANG\" ;;
    esac
    # Allowlist: only keep characters valid in a filesystem path to prevent
    # injection if the shebang contains shell metacharacters
    case \"$GRAPHIFY_PYTHON\" in
        *[!a-zA-Z0-9/_.@-]*) GRAPHIFY_PYTHON=\"\" ;;
    esac
    if [ -n \"$GRAPHIFY_PYTHON\" ] && ! \"$GRAPHIFY_PYTHON\" -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"\"
    fi
fi
# Fall back: try python3, then python (Windows has no python3 shim)
if [ -z \"$GRAPHIFY_PYTHON\" ]; then
    if command -v python3 >/dev/null 2>&1 && python3 -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python3\"
    elif command -v python >/dev/null 2>&1 && python -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python\"
    else
        exit 0
    fi
fi

export GRAPHIFY_CHANGED=\"$CHANGED\"

# Run rebuild detached so git commit returns immediately.
# Full repo rebuilds can take hours; blocking the post-commit hook stalls the shell.
_GRAPHIFY_LOG=\"${HOME}/.cache/graphify-rebuild.log\"
mkdir -p \"$(dirname \"$_GRAPHIFY_LOG\")\"
echo \"[graphify hook] launching background rebuild (log: $_GRAPHIFY_LOG)\"
nohup $GRAPHIFY_PYTHON -c \"
import os, signal, sys
from pathlib import Path

changed_raw = os.environ.get('GRAPHIFY_CHANGED', '')
changed = [Path(f.strip()) for f in changed_raw.strip().splitlines() if f.strip()]

if not changed:
    sys.exit(0)

print(f'[graphify hook] {len(changed)} file(s) changed - rebuilding graph...')

try:
    from graphify.watch import _rebuild_code, _apply_resource_limits
    _apply_resource_limits()
    _timeout = int(os.environ.get('GRAPHIFY_REBUILD_TIMEOUT', '600'))
    if _timeout > 0 and hasattr(signal, 'SIGALRM'):
        signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError(f'graphify rebuild exceeded {_timeout}s')))
        signal.alarm(_timeout)
    _force = os.environ.get('GRAPHIFY_FORCE', '').lower() in ('1', 'true', 'yes')
    _rebuild_code(Path('.'), changed_paths=changed, force=_force)
except TimeoutError as exc:
    print(f'[graphify hook] {exc}')
    sys.exit(1)
except Exception as exc:
    print(f'[graphify hook] Rebuild failed: {exc}')
    sys.exit(1)
\" > \"$_GRAPHIFY_LOG\" 2>&1 < /dev/null &
disown 2>/dev/null || true
# graphify-hook-end
";

/// The full post-checkout hook script, byte-identical to Python's `_CHECKOUT_SCRIPT`.
pub const CHECKOUT_SCRIPT: &str = "# graphify-checkout-hook-start
# Auto-rebuilds the knowledge graph (code only) when switching branches.
# Installed by: graphify hook install

PREV_HEAD=$1
NEW_HEAD=$2
BRANCH_SWITCH=$3

# Only run on branch switches, not file checkouts
if [ \"$BRANCH_SWITCH\" != \"1\" ]; then
    exit 0
fi

# Only run if graphify-out/ exists (graph has been built before)
if [ ! -d \"graphify-out\" ]; then
    exit 0
fi

# Skip during rebase/merge/cherry-pick
GIT_DIR=$(git rev-parse --git-dir 2>/dev/null)
[ -d \"$GIT_DIR/rebase-merge\" ] && exit 0
[ -d \"$GIT_DIR/rebase-apply\" ] && exit 0
[ -f \"$GIT_DIR/MERGE_HEAD\" ] && exit 0
[ -f \"$GIT_DIR/CHERRY_PICK_HEAD\" ] && exit 0

# Detect the correct Python interpreter (handles pipx, venv, system installs)
GRAPHIFY_BIN=$(command -v graphify 2>/dev/null)
if [ -n \"$GRAPHIFY_BIN\" ]; then
    case \"$GRAPHIFY_BIN\" in
        *.exe) _SHEBANG=\"\" ;;
        *)     _SHEBANG=$(head -1 \"$GRAPHIFY_BIN\" | sed 's/^#![[:space:]]*//') ;;
    esac
    case \"$_SHEBANG\" in
        */env\\ *) GRAPHIFY_PYTHON=\"${_SHEBANG#*/env }\" ;;
        *)         GRAPHIFY_PYTHON=\"$_SHEBANG\" ;;
    esac
    # Allowlist: only keep characters valid in a filesystem path to prevent
    # injection if the shebang contains shell metacharacters
    case \"$GRAPHIFY_PYTHON\" in
        *[!a-zA-Z0-9/_.@-]*) GRAPHIFY_PYTHON=\"\" ;;
    esac
    if [ -n \"$GRAPHIFY_PYTHON\" ] && ! \"$GRAPHIFY_PYTHON\" -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"\"
    fi
fi
# Fall back: try python3, then python (Windows has no python3 shim)
if [ -z \"$GRAPHIFY_PYTHON\" ]; then
    if command -v python3 >/dev/null 2>&1 && python3 -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python3\"
    elif command -v python >/dev/null 2>&1 && python -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python\"
    else
        exit 0
    fi
fi

_GRAPHIFY_LOG=\"${HOME}/.cache/graphify-rebuild.log\"
mkdir -p \"$(dirname \"$_GRAPHIFY_LOG\")\"
echo \"[graphify] Branch switched - launching background rebuild (log: $_GRAPHIFY_LOG)\"
nohup $GRAPHIFY_PYTHON -c \"
from graphify.watch import _rebuild_code, _apply_resource_limits
from pathlib import Path
import os, signal, sys
try:
    _apply_resource_limits()
    _timeout = int(os.environ.get('GRAPHIFY_REBUILD_TIMEOUT', '600'))
    if _timeout > 0 and hasattr(signal, 'SIGALRM'):
        signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError(f'graphify rebuild exceeded {_timeout}s')))
        signal.alarm(_timeout)
    _force = os.environ.get('GRAPHIFY_FORCE', '').lower() in ('1', 'true', 'yes')
    # post-checkout: branch switch can touch arbitrary files; full rebuild path
    # (no changed_paths) is correct here. The flock inside _rebuild_code still
    # prevents pile-ups when commit + checkout fire back-to-back.
    _rebuild_code(Path('.'), force=_force)
except TimeoutError as exc:
    print(f'[graphify] {exc}')
    sys.exit(1)
except Exception as exc:
    print(f'[graphify] Rebuild failed: {exc}')
    sys.exit(1)
\" > \"$_GRAPHIFY_LOG\" 2>&1 < /dev/null &
disown 2>/dev/null || true
# graphify-checkout-hook-end
";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from hook installation/uninstallation.
#[derive(Debug, Error)]
pub enum HooksError {
    /// No git repository found at or above the given path.
    #[error("No git repository found at or above {0}")]
    NotAGitRepo(PathBuf),

    /// Filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation/deserialisation error.
    #[error("JSON error: {0}")]
    Json(String),

    /// Unknown platform name passed to `install_platform_skill`.
    #[error("unknown platform '{0}'")]
    UnknownPlatform(String),
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Walk upward from `path` to find the nearest `.git` directory.
fn git_root(path: &Path) -> Option<PathBuf> {
    let current = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut candidate = current.as_path();
    loop {
        if candidate.join(".git").exists() {
            return Some(candidate.to_path_buf());
        }
        match candidate.parent() {
            Some(p) => candidate = p,
            None => return None,
        }
    }
}

/// Resolve the git hooks directory for `root`, respecting `core.hooksPath`.
///
/// The `rev_parse_fn` parameter is injected so tests can substitute a mock
/// instead of spawning a real git process.  It receives the repository root
/// and returns the raw stdout from `git -C <root> rev-parse --git-path hooks`
/// (or `None` on failure).
///
/// # Errors
///
/// Returns `HooksError::Io` if the resolved hooks directory cannot be created.
pub fn hooks_dir(root: &Path) -> Result<PathBuf, HooksError> {
    hooks_dir_with(root, &default_rev_parse)
}

/// Like [`hooks_dir`] but accepts an injectable `rev_parse_fn` for testing.
///
/// # Errors
///
/// Returns `HooksError::Io` if the resolved hooks directory cannot be created.
pub fn hooks_dir_with(
    root: &Path,
    rev_parse_fn: &dyn Fn(&Path) -> Option<String>,
) -> Result<PathBuf, HooksError> {
    // --- Step 1: try core.hooksPath from .git/config (mirrors configparser logic) ---
    let git_config = root.join(".git").join("config");
    if let Ok(content) = fs::read_to_string(&git_config)
        && let Some(custom) = parse_hooks_path(&content)
    {
        let p = PathBuf::from(shellexpand::tilde(&custom).as_ref());
        let p = if p.is_absolute() { p } else { root.join(&p) };
        // Validate the resolved path stays within the repository root
        // to prevent supply-chain attacks via malicious core.hooksPath values.
        // Create dir first so canonicalize can resolve symlinks (e.g. macOS /var→/private/var).
        let root_canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        // Resolve p without creating it yet (using a best-effort normalize).
        // We create the dir *after* the security check.
        let p_for_check = p.canonicalize().unwrap_or_else(|_| p.clone());
        if p_for_check.starts_with(&root_canonical) {
            fs::create_dir_all(&p)?;
            return Ok(p.canonicalize().unwrap_or(p));
        }
        // Path escapes repo root — fall through to default
    }

    // --- Step 2: git rev-parse --git-path hooks (handles linked worktrees) ---
    if let Some(raw) = rev_parse_fn(root) {
        let raw = raw.trim().to_string();
        // A valid hooks path can never contain newlines or NUL.
        if !raw.is_empty() && !raw.contains('\n') && !raw.contains('\r') && !raw.contains('\x00') {
            let d = root.join(&raw);
            // Create dir first so canonicalize succeeds (resolves macOS /var → /private/var symlinks).
            fs::create_dir_all(&d)?;
            return Ok(d.canonicalize().unwrap_or(d));
        }
    }

    // --- Step 3: default fallback ---
    let d = root.join(".git").join("hooks");
    fs::create_dir_all(&d)?;
    Ok(d.canonicalize().unwrap_or(d))
}

/// Default `rev_parse_fn` that shells out to git.
fn default_rev_parse(root: &Path) -> Option<String> {
    let res = Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "rev-parse",
            "--git-path",
            "hooks",
        ])
        .output()
        .ok()?;
    if res.status.success() {
        Some(String::from_utf8_lossy(&res.stdout).into_owned())
    } else {
        None
    }
}

/// Parse `core.hooksPath` (case-insensitive key) from raw `.git/config` text.
/// Returns the trimmed value, or `None` if not present / empty.
///
/// `configparser` lowercases option names, so git's `hooksPath` becomes
/// `hookspath` — we match both forms.
fn parse_hooks_path(config_text: &str) -> Option<String> {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        // SAFETY: literal pattern is valid.
        #[allow(clippy::unwrap_used)]
        Regex::new(r"(?i)^hookspath\s*=\s*(.+)$").unwrap()
    });

    let mut in_core = false;
    for line in config_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // Section header: [core] or [core "..."]
            in_core = trimmed.to_lowercase().starts_with("[core");
            continue;
        }
        if in_core && let Some(caps) = RE.captures(trimmed) {
            let val = caps[1].trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Install a single git hook (appending if an existing hook is present).
/// Returns a human-readable result message.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
fn install_hook(
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

/// Remove graphify section from a git hook using start/end markers.
/// Returns a human-readable result message.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
fn uninstall_hook(
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

    // Build the pattern from runtime marker values (escaped, so valid regex).
    // SAFETY: pattern is constructed from escaped literals; cannot be invalid.
    #[allow(clippy::unwrap_used)]
    let re = Regex::new(&format!(
        "(?s){}.*?{}\n?",
        regex::escape(marker),
        regex::escape(marker_end)
    ))
    .unwrap();

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

/// Set executable bits on Unix; no-op on other platforms.
fn set_executable(path: &Path) -> Result<(), HooksError> {
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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Install graphify post-commit and post-checkout hooks in the nearest git repo.
///
/// # Errors
///
/// Returns `HooksError::NotAGitRepo` if no git repository is found at or above
/// `path`, or `HooksError::Io` on filesystem failures.
pub fn install(path: &Path) -> Result<String, HooksError> {
    let root = git_root(path).ok_or_else(|| {
        let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        HooksError::NotAGitRepo(resolved)
    })?;

    let hdir = hooks_dir(&root)?;

    let commit_msg = install_hook(&hdir, "post-commit", HOOK_SCRIPT, HOOK_MARKER)?;
    let checkout_msg = install_hook(&hdir, "post-checkout", CHECKOUT_SCRIPT, CHECKOUT_MARKER)?;

    Ok(format!(
        "post-commit: {commit_msg}\npost-checkout: {checkout_msg}"
    ))
}

/// Remove graphify post-commit and post-checkout hooks.
///
/// # Errors
///
/// Returns `HooksError::NotAGitRepo` if no git repository is found at or above
/// `path`, or `HooksError::Io` on filesystem failures.
pub fn uninstall(path: &Path) -> Result<String, HooksError> {
    let root = git_root(path).ok_or_else(|| {
        let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        HooksError::NotAGitRepo(resolved)
    })?;

    let hdir = hooks_dir(&root)?;

    let commit_msg = uninstall_hook(&hdir, "post-commit", HOOK_MARKER, HOOK_MARKER_END)?;
    let checkout_msg =
        uninstall_hook(&hdir, "post-checkout", CHECKOUT_MARKER, CHECKOUT_MARKER_END)?;

    Ok(format!(
        "post-commit: {commit_msg}\npost-checkout: {checkout_msg}"
    ))
}

/// Check whether graphify hooks are installed.
///
/// Returns a human-readable status string. Never fails — returns
/// `"Not in a git repository."` if outside a repo.
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

/// Check the status of a single hook file.
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
