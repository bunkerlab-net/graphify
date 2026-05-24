//! Public shell-script constants for the post-commit / post-checkout git hooks.
//!
//! The strings here are byte-identical to their Python counterparts so an
//! installed hook produced by the Rust binary matches the Python reference
//! exactly. Tests assert on the contents.

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
