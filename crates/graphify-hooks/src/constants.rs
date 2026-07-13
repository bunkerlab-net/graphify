//! Public shell-script constants for the post-commit / post-checkout git hooks.
//!
//! These mirror their Python counterparts so an installed hook produced by the
//! Rust binary matches the Python reference. Intentional divergences:
//! - A 1 MiB cap on the rebuild log: graphify-py appends to
//!   `~/.cache/graphify-rebuild.log` unbounded, which grows without limit
//!   across commits; the Rust hooks truncate to the most recent 1 MiB first.
//! - `GRAPHIFY_SKIP_HOOK=1` is honoured by *both* hooks; graphify-py only wired
//!   it into post-commit, so its post-checkout hook ignored the opt-out.
//!
//! The `__PINNED_PYTHON__` placeholder is substituted at install time (with the
//! empty string for the Rust binary, which has no `sys.executable` to pin), so
//! the hook falls through to runtime interpreter detection.
//!
//! Tests assert on the contents.

/// Start marker for the post-commit hook section.
pub const HOOK_MARKER: &str = "# graphify-hook-start";
/// End marker for the post-commit hook section.
pub const HOOK_MARKER_END: &str = "# graphify-hook-end";
/// Start marker for the post-checkout hook section.
pub const CHECKOUT_MARKER: &str = "# graphify-checkout-hook-start";
/// End marker for the post-checkout hook section.
pub const CHECKOUT_MARKER_END: &str = "# graphify-checkout-hook-end";

/// The interpreter-detection block as a string literal, shared by
/// [`PYTHON_DETECT`] and embedded verbatim into both hook scripts via `concat!`
/// so the three stay byte-identical without manual duplication.
macro_rules! python_detect_block {
    () => {
        "\
# Detect the correct Python interpreter (handles uv tool, pipx, venv, system installs).
# _PINNED was recorded at hook-install time; tried first so the hook works even
# when the graphify launcher is not on PATH (common in GUI clients and CI).
GRAPHIFY_PYTHON=\"\"
_PINNED='__PINNED_PYTHON__'
if [ -n \"$_PINNED\" ] && [ -x \"$_PINNED\" ] && \"$_PINNED\" -c \"import graphify\" 2>/dev/null; then
    GRAPHIFY_PYTHON=\"$_PINNED\"
fi
# Second probe: read graphify-out/.graphify_python (written by the skill and CLI;
# survives uv-tool reinstalls and is the same source the README documents).
if [ -z \"$GRAPHIFY_PYTHON\" ]; then
    _GFY_PYTHON_FILE=\"graphify-out/.graphify_python\"
    if [ -f \"$_GFY_PYTHON_FILE\" ]; then
        _FROM_FILE=$(cat \"$_GFY_PYTHON_FILE\" 2>/dev/null | tr -d '[:space:]')
        case \"$_FROM_FILE\" in
            *[!a-zA-Z0-9/_.@:\\-]*) _FROM_FILE=\"\" ;;
        esac
        if [ -n \"$_FROM_FILE\" ] && [ -x \"$_FROM_FILE\" ] && \"$_FROM_FILE\" -c \"import graphify\" 2>/dev/null; then
            GRAPHIFY_PYTHON=\"$_FROM_FILE\"
        fi
    fi
fi
# Third probe: resolve via the graphify launcher on PATH (shebang probe).
if [ -z \"$GRAPHIFY_PYTHON\" ]; then
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
        case \"$GRAPHIFY_PYTHON\" in
            *[!a-zA-Z0-9/_.@-]*) GRAPHIFY_PYTHON=\"\" ;;
        esac
        if [ -n \"$GRAPHIFY_PYTHON\" ] && ! \"$GRAPHIFY_PYTHON\" -c \"import graphify\" 2>/dev/null; then
            GRAPHIFY_PYTHON=\"\"
        fi
    fi
fi
# Last resort: try python3 / python (works for system/venv installs on PATH).
if [ -z \"$GRAPHIFY_PYTHON\" ]; then
    if command -v python3 >/dev/null 2>&1 && python3 -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python3\"
    elif command -v python >/dev/null 2>&1 && python -c \"import graphify\" 2>/dev/null; then
        GRAPHIFY_PYTHON=\"python\"
    else
        echo \"[graphify hook] could not locate a Python with graphify installed. Add the graphify bin dir to PATH or re-run 'graphify hook install' from the env where graphify lives.\" >&2
        exit 0
    fi
fi
"
    };
}

/// Shell snippet that detects the correct Python interpreter (4 probes:
/// pinned interpreter, `graphify-out/.graphify_python`, launcher shebang, then
/// `python3`/`python`). Embedded verbatim into both hook scripts.
pub const PYTHON_DETECT: &str = python_detect_block!();

/// The 1 MiB rebuild-log cap snippet (Rust divergence; graphify-py appends
/// unbounded).
macro_rules! log_cap_block {
    () => {
        "\
# Cap the rebuild log so the append below can't grow it without bound; keep the
# most recent 1 MiB. (Divergence from graphify-py, which appends unbounded.)
_GRAPHIFY_LOG_MAX_BYTES=1048576
if [ -f \"$_GRAPHIFY_LOG\" ] && [ \"$(wc -c < \"$_GRAPHIFY_LOG\" 2>/dev/null || echo 0)\" -gt \"$_GRAPHIFY_LOG_MAX_BYTES\" ]; then
    { tail -c \"$_GRAPHIFY_LOG_MAX_BYTES\" \"$_GRAPHIFY_LOG\" > \"$_GRAPHIFY_LOG.tmp\" 2>/dev/null && mv -f \"$_GRAPHIFY_LOG.tmp\" \"$_GRAPHIFY_LOG\"; } || rm -f \"$_GRAPHIFY_LOG.tmp\"
fi
"
    };
}

/// Cap hook-triggered rebuild parallelism on Git for Windows / MSYS (#879c058).
/// Those shells inherit fragile pipe handles from GUI clients and agent shells,
/// so default rebuilds to sequential; an explicit `GRAPHIFY_MAX_WORKERS` wins.
macro_rules! windows_worker_cap_block {
    () => {
        "\
# Git for Windows/MSYS hooks can inherit fragile pipe handles from GUI clients
# and agent shells. Keep hook-triggered rebuilds sequential by default there;
# explicit GRAPHIFY_MAX_WORKERS still wins for users who want parallelism.
if [ -n \"${WINDIR:-}\" ] || [ -n \"${MSYSTEM:-}\" ]; then
    export GRAPHIFY_MAX_WORKERS=\"${GRAPHIFY_MAX_WORKERS:-1}\"
fi

"
    };
}

/// The full post-commit hook script. Mirrors Python's `_HOOK_SCRIPT` (with the
/// `_detached_launch` launcher expanded inline) except for the 1 MiB
/// rebuild-log cap (see the module-level note).
pub const HOOK_SCRIPT: &str = concat!(
    "# graphify-hook-start
# Auto-rebuilds the knowledge graph after each commit (code files only, no LLM needed).
# Installed by: graphify hook install

# Deterministic clustering: networkx louvain iterates string-keyed sets whose
# order is randomized per-process by PYTHONHASHSEED, so community assignments
# churn run-to-run. Pinning it makes graphify-out reproducible.
export PYTHONHASHSEED=0

# Skip during rebase/merge/cherry-pick to avoid blocking --continue with unstaged changes
GIT_DIR=$(git rev-parse --git-dir 2>/dev/null)
[ -d \"$GIT_DIR/rebase-merge\" ] && exit 0
[ -d \"$GIT_DIR/rebase-apply\" ] && exit 0
[ -f \"$GIT_DIR/MERGE_HEAD\" ] && exit 0
[ -f \"$GIT_DIR/CHERRY_PICK_HEAD\" ] && exit 0

[ \"${GRAPHIFY_SKIP_HOOK:-0}\" = \"1\" ] && exit 0

CHANGED=$(git diff --name-only HEAD~1 HEAD 2>/dev/null || git diff --name-only HEAD 2>/dev/null)
if [ -z \"$CHANGED\" ]; then
    exit 0
fi

# Skip when only graphify-out/ artifacts changed (avoids rebuild loop when graph outputs are tracked in git)
_NON_GRAPH=$(echo \"$CHANGED\" | grep -v '^graphify-out/' || true)
if [ -z \"$_NON_GRAPH\" ]; then
    exit 0
fi

",
    windows_worker_cap_block!(),
    python_detect_block!(),
    "
export GRAPHIFY_CHANGED=\"$CHANGED\"

# Run the rebuild detached so git commit returns immediately. Full-repo rebuilds
# can take hours; blocking the post-commit hook stalls the shell. The Python
# launcher below detaches the child cross-platform, so it works on Git for
# Windows' shell too (which lacks the coreutils backgrounding tools) (#1161).
_GRAPHIFY_LOG=\"${HOME}/.cache/graphify-rebuild.log\"
mkdir -p \"$(dirname \"$_GRAPHIFY_LOG\")\"
export GRAPHIFY_REBUILD_LOG=\"$_GRAPHIFY_LOG\"
",
    log_cap_block!(),
    "echo \"[graphify hook] launching background rebuild (log: $_GRAPHIFY_LOG)\"
\"$GRAPHIFY_PYTHON\" -c \"import os, subprocess, sys
_src = '''
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
    _root = Path('.')
    _out = os.environ.get('GRAPHIFY_OUT', 'graphify-out')
    _saved = Path(_out) / '.graphify_root'
    if _saved.exists():
        _txt = _saved.read_text(encoding='utf-8').strip()
        if _txt:
            _root = Path(_txt)
    _rebuild_code(_root, changed_paths=changed, force=_force)
    # Refresh the work-memory lessons doc when saved Q&A outcomes exist
    # (best-effort; never fails the hook).
    try:
        _md = (_root / _out) / 'memory'
        if _md.is_dir() and any(_md.glob('*.md')):
            from graphify.reflect import reflect as _reflect
            _gj = (_root / _out) / 'graph.json'
            _reflect(memory_dir=_md, out_path=(_root / _out) / 'reflections' / 'LESSONS.md',
                     graph_path=_gj if _gj.exists() else None)
    except Exception:
        pass
except TimeoutError as exc:
    print(f'[graphify hook] {exc}')
    sys.exit(1)
except Exception as exc:
    print(f'[graphify hook] Rebuild failed: {exc}')
    sys.exit(1)
'''
_log = os.environ.get('GRAPHIFY_REBUILD_LOG') or os.path.join(os.path.expanduser('~'), '.cache', 'graphify-rebuild.log')
try:
    os.makedirs(os.path.dirname(_log), exist_ok=True)
    _out = open(_log, 'a', buffering=1, encoding='utf-8', errors='replace')
except OSError:
    _out = subprocess.DEVNULL
_kw = dict(stdout=_out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, cwd=os.getcwd(), close_fds=True)
_cmd = [sys.executable, '-c', _src]
if os.name == 'nt':
    _flags = 0x00000008 | 0x00000200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    try:
        subprocess.Popen(_cmd, creationflags=_flags | 0x01000000, **_kw)  # + CREATE_BREAKAWAY_FROM_JOB
    except OSError:
        subprocess.Popen(_cmd, creationflags=_flags, **_kw)
else:
    subprocess.Popen(_cmd, start_new_session=True, **_kw)
\"
# graphify-hook-end
"
);

/// The full post-checkout hook script. Mirrors Python's `_CHECKOUT_SCRIPT`
/// (with the `_detached_launch` launcher expanded inline) except for the 1 MiB
/// rebuild-log cap (see the module-level note).
pub const CHECKOUT_SCRIPT: &str = concat!(
    "# graphify-checkout-hook-start
# Auto-rebuilds the knowledge graph (code only) when switching branches.
# Installed by: graphify hook install

# Deterministic clustering: networkx louvain iterates string-keyed sets whose
# order is randomized per-process by PYTHONHASHSEED, so community assignments
# churn run-to-run. Pinning it makes graphify-out reproducible.
export PYTHONHASHSEED=0

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

[ \"${GRAPHIFY_SKIP_HOOK:-0}\" = \"1\" ] && exit 0

",
    windows_worker_cap_block!(),
    python_detect_block!(),
    "
_GRAPHIFY_LOG=\"${HOME}/.cache/graphify-rebuild.log\"
mkdir -p \"$(dirname \"$_GRAPHIFY_LOG\")\"
export GRAPHIFY_REBUILD_LOG=\"$_GRAPHIFY_LOG\"
",
    log_cap_block!(),
    "echo \"[graphify] Branch switched - launching background rebuild (log: $_GRAPHIFY_LOG)\"
\"$GRAPHIFY_PYTHON\" -c \"import os, subprocess, sys
_src = '''
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
    _root = Path('.')
    _out = os.environ.get('GRAPHIFY_OUT', 'graphify-out')
    _saved = Path(_out) / '.graphify_root'
    if _saved.exists():
        _txt = _saved.read_text(encoding='utf-8').strip()
        if _txt:
            _root = Path(_txt)
    _rebuild_code(_root, force=_force)
    # Refresh the work-memory lessons doc when saved Q&A outcomes exist
    # (best-effort; never fails the hook).
    try:
        _md = (_root / _out) / 'memory'
        if _md.is_dir() and any(_md.glob('*.md')):
            from graphify.reflect import reflect as _reflect
            _gj = (_root / _out) / 'graph.json'
            _reflect(memory_dir=_md, out_path=(_root / _out) / 'reflections' / 'LESSONS.md',
                     graph_path=_gj if _gj.exists() else None)
    except Exception:
        pass
except TimeoutError as exc:
    print(f'[graphify] {exc}')
    sys.exit(1)
except Exception as exc:
    print(f'[graphify] Rebuild failed: {exc}')
    sys.exit(1)
'''
_log = os.environ.get('GRAPHIFY_REBUILD_LOG') or os.path.join(os.path.expanduser('~'), '.cache', 'graphify-rebuild.log')
try:
    os.makedirs(os.path.dirname(_log), exist_ok=True)
    _out = open(_log, 'a', buffering=1, encoding='utf-8', errors='replace')
except OSError:
    _out = subprocess.DEVNULL
_kw = dict(stdout=_out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, cwd=os.getcwd(), close_fds=True)
_cmd = [sys.executable, '-c', _src]
if os.name == 'nt':
    _flags = 0x00000008 | 0x00000200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    try:
        subprocess.Popen(_cmd, creationflags=_flags | 0x01000000, **_kw)  # + CREATE_BREAKAWAY_FROM_JOB
    except OSError:
        subprocess.Popen(_cmd, creationflags=_flags, **_kw)
else:
    subprocess.Popen(_cmd, start_new_session=True, **_kw)
\"
# graphify-checkout-hook-end
"
);
