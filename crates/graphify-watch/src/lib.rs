//! Filesystem watcher that rebuilds the graph on file changes.
//!
//! Ports `graphify-py/graphify/watch.py`.
//!
//! # Architecture
//!
//! The primary entry point for production use is [`watch`], which spawns a
//! [`notify_debouncer_full`] watcher and batches events over a configurable
//! debounce window.  The full rebuild pipeline lives in [`rebuild`].

pub mod canonical;
pub mod error;
pub mod lock;
pub mod rebuild;

pub use error::WatchError;
pub use lock::RebuildLock;
pub use rebuild::{check_shrink, git_head, node_community_map, relativize_source_files};

use std::path::{Path, PathBuf};
use std::time::Duration;

use graphify_detect::extensions::CODE_EXTENSIONS;
use graphify_detect::{is_ignored, load_graphifyignore};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default output sub-directory name, overridden by `GRAPHIFY_OUT`.
const DEFAULT_GRAPHIFY_OUT: &str = "graphify-out";

/// Return the effective output directory name from the environment.
#[must_use]
pub fn graphify_out() -> String {
    std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| DEFAULT_GRAPHIFY_OUT.to_string())
}

// ── Extension sets ────────────────────────────────────────────────────────────

/// All extensions that the watcher pays attention to (code + doc + paper + image).
///
/// Corresponds to `_WATCHED_EXTENSIONS` in Python.
/// Elements are bare extensions **without** a leading dot, matching
/// `graphify_detect::extensions::CODE_EXTENSIONS` et al.
pub const WATCHED_EXTENSIONS: &[&str] = {
    // Built as a concatenation of the four upstream slices.
    // In const context we cannot call heap-allocating helpers, so we list the
    // combined set explicitly, keeping it in sync with the detect crate slices.
    &[
        // CODE_EXTENSIONS
        "py", "ts", "js", "jsx", "tsx", "mjs", "ejs", "go", "rs", "java", "groovy", "gradle", "cpp",
        "cc", "cxx", "c", "h", "hpp", "rb", "swift", "kt", "kts", "cs", "scala", "php", "lua",
        "luau", "toc", "zig", "ps1", "ex", "exs", "m", "mm", "jl", "vue", "svelte", "astro",
        "dart", "v", "sv", "sql", "r", "f", "F", "f90", "F90", "f95", "F95", "f03", "F03", "f08",
        "F08", "pas", "pp", "dpr", "dpk", "lpr", "inc", "dfm", "lfm", "lpk", "sh", "bash", "json",
        // DOC_EXTENSIONS
        "md", "mdx", "qmd", "txt", "rst", "html", "yaml", "yml", // PAPER_EXTENSIONS
        "pdf", // IMAGE_EXTENSIONS
        "png", "jpg", "jpeg", "gif", "webp", "svg",
    ]
};

// ── notify_only ───────────────────────────────────────────────────────────────

/// Write a `needs_update` flag file and print a notification.
///
/// Called for non-code file changes (docs, papers, images) that require LLM
/// re-extraction via `graphify --update`.
///
/// Ports `_notify_only` from Python.
///
/// # Errors
///
/// Returns `WatchError::Io` if the flag file cannot be created.
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

// ── check_update ──────────────────────────────────────────────────────────────

/// Check for a pending semantic update flag and notify the user if set.
///
/// Always returns `true` so cron jobs do not alarm.  Non-code file changes
/// require LLM-backed re-extraction — this function only signals that the
/// update is needed.
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

// ── apply_resource_limits ─────────────────────────────────────────────────────

/// Apply best-effort nice(10) + optional RSS memory cap.
///
/// Reads `GRAPHIFY_REBUILD_MEMORY_LIMIT_MB` from the environment.
/// Uses `RLIMIT_DATA` on macOS and `RLIMIT_AS` on Linux, silently skipping
/// when the platform does not support it.
///
/// Ports `_apply_resource_limits` from Python.
///
/// Called from hook shell scripts — the Python entrypoint is referenced by
/// those scripts and this Rust equivalent must exist at the same symbol path.
///
/// # Note
///
/// Full resource-limit support is deferred pending stabilisation of platform
/// bindings.  The current implementation is a no-op placeholder; see
/// `.claude/local/notes/module_watch.md`.
#[allow(clippy::missing_panics_doc)] // reason: this function never panics
pub fn apply_resource_limits() {
    // Best-effort only — failures are silently swallowed, matching Python.
    #[cfg(unix)]
    {
        // SAFETY: nice(2) is always safe to call; we ignore the return value.
        #[allow(unsafe_code)] // reason: libc::nice has no safe Rust wrapper
        unsafe {
            libc::nice(10);
        }
        let mb_str = std::env::var("GRAPHIFY_REBUILD_MEMORY_LIMIT_MB").unwrap_or_default();
        let mb_str = mb_str.trim();
        if mb_str.is_empty() {
            return;
        }
        let Ok(mb) = mb_str.parse::<u64>() else {
            return;
        };
        let limit = mb * 1024 * 1024;
        // SAFETY: setrlimit is safe to call with valid resource constants.
        #[allow(unsafe_code)] // reason: libc::setrlimit/getrlimit have no safe Rust wrapper
        unsafe {
            #[cfg(target_os = "macos")]
            let resource = libc::RLIMIT_DATA;
            #[cfg(not(target_os = "macos"))]
            let resource = libc::RLIMIT_AS;

            let mut rl = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(resource, &raw mut rl) == 0 {
                let new_hard = if rl.rlim_max != libc::RLIM_INFINITY && rl.rlim_max < limit {
                    rl.rlim_max
                } else {
                    limit
                };
                let new_rl = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: new_hard,
                };
                libc::setrlimit(resource, &raw const new_rl);
            }
        }
    }
}

// ── rebuild_code ──────────────────────────────────────────────────────────────

/// Re-run AST extraction + build + optional cluster + report for code files.
///
/// Acquires a per-repo advisory lock (unless `acquire_lock` is `false`).
/// Returns `Ok(true)` when outputs were updated, `Ok(false)` when the rebuild
/// was skipped (lock held, no tracked files changed, shrink guard refused).
///
/// Ports `_rebuild_code` from Python.
///
/// # Errors
///
/// Returns `WatchError` on pipeline failure.
#[allow(clippy::fn_params_excessive_bools)]
// reason: mirrors Python's `_rebuild_code` signature byte-for-byte; each
// bool controls a distinct pipeline flag and extracting enums would diverge
// from the Python reference spec.
pub fn rebuild_code(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    follow_symlinks: bool,
    force: bool,
    no_cluster: bool,
    acquire_lock: bool,
    block_on_lock: bool,
) -> Result<bool, WatchError> {
    rebuild::rebuild_code(
        watch_path,
        changed_paths,
        follow_symlinks,
        force,
        no_cluster,
        acquire_lock,
        block_on_lock,
    )
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Return `true` if any of the paths has an extension not in `CODE_EXTENSIONS`.
#[must_use]
fn has_non_code(changed_paths: &[PathBuf]) -> bool {
    changed_paths.iter().any(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_none_or(|e| !CODE_EXTENSIONS.contains(&e))
    })
}

/// Return `true` if any of the paths has an extension in `CODE_EXTENSIONS`.
#[must_use]
fn has_code(changed_paths: &[PathBuf]) -> bool {
    changed_paths.iter().any(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| CODE_EXTENSIONS.contains(&e))
    })
}

// ── watch ─────────────────────────────────────────────────────────────────────

/// Watch `watch_path` for file changes and auto-update the graph.
///
/// - Code-only changes: calls [`rebuild_code`] (currently panics — see note).
/// - Doc/paper/image changes: calls [`notify_only`].
///
/// Uses `notify-debouncer-full` to batch rapid saves over `debounce` seconds.
/// Blocks until interrupted (Ctrl-C / `SIGINT`).
///
/// Ports `watch` from Python (which used `watchdog`).
///
/// # Errors
///
/// Returns `WatchError::Notify` if the underlying watcher cannot be
/// initialised or if filesystem events cannot be received.
pub fn watch(watch_path: &Path, debounce: f64) -> Result<(), WatchError> {
    use notify_debouncer_full::{DebounceEventResult, new_debouncer, notify::RecursiveMode};

    let debounce_dur = Duration::from_secs_f64(debounce);
    let out_dir_name = graphify_out();

    // Load .graphifyignore patterns ONCE at startup (mirrors gh-928 fix).
    let watch_root = watch_path
        .canonicalize()
        .unwrap_or_else(|_| watch_path.to_path_buf());
    let ignore_patterns = load_graphifyignore(&watch_root);

    let (tx, rx) = std::sync::mpsc::channel::<DebounceEventResult>();

    let mut debouncer = new_debouncer(debounce_dur, None, tx).map_err(WatchError::Notify)?;

    // notify-debouncer-full 0.7+ removed the `.watcher()` accessor; the
    // `Debouncer` now implements `Watcher` directly, so we call `.watch()`
    // on the debouncer itself.
    debouncer
        .watch(watch_path, RecursiveMode::Recursive)
        .map_err(WatchError::Notify)?;

    println!(
        "[graphify watch] Watching {} - press Ctrl+C to stop",
        watch_root.display()
    );
    println!(
        "[graphify watch] Code changes rebuild graph automatically. \
         Doc/image changes require /graphify --update."
    );
    println!("[graphify watch] Debounce: {debounce}s");

    while let Ok(result) = rx.recv() {
        let events = match result {
            Ok(evs) => evs,
            Err(errs) => {
                for e in &errs {
                    eprintln!("[graphify watch] watcher error: {e}");
                }
                continue;
            }
        };

        // Collect changed paths, applying the same filters as the Python handler.
        let changed: Vec<PathBuf> = events
            .into_iter()
            .flat_map(|ev| ev.event.paths)
            .filter(|p| {
                // Skip directories.
                if p.is_dir() {
                    return false;
                }
                // Apply .graphifyignore.
                if !ignore_patterns.is_empty() && is_ignored(p, &watch_root, &ignore_patterns) {
                    return false;
                }
                // Check extension.
                let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
                    return false;
                };
                if !WATCHED_EXTENSIONS.contains(&ext) {
                    return false;
                }
                // Skip dot-prefixed path segments.
                if p.components()
                    .any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.')))
                {
                    return false;
                }
                // Skip the graphify-out directory.
                if p.components()
                    .any(|c| c.as_os_str() == out_dir_name.as_str())
                {
                    return false;
                }
                true
            })
            .collect();

        if changed.is_empty() {
            continue;
        }

        println!("\n[graphify watch] {} file(s) changed", changed.len());

        if has_code(&changed) {
            match rebuild_code(watch_path, Some(&changed), false, false, false, true, false) {
                Ok(true) => println!("[graphify watch] graph rebuilt successfully."),
                Ok(false) => {
                    println!("[graphify watch] rebuild skipped (lock held or no changes).");
                }
                Err(e) => eprintln!("[graphify watch] rebuild failed: {e}"),
            }
        }
        if has_non_code(&changed)
            && let Err(e) = notify_only(watch_path)
        {
            eprintln!("[graphify watch] notify_only failed: {e}");
        }
    }

    Ok(())
}
