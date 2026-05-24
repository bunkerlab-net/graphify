//! Filesystem watcher entry point and helpers.

use std::path::{Path, PathBuf};
use std::time::Duration;

use graphify_detect::extensions::CODE_EXTENSIONS;
use graphify_detect::{is_ignored, load_graphifyignore};

use crate::constants::WATCHED_EXTENSIONS;
use crate::error::WatchError;
use crate::notify::{graphify_out, notify_only};
use crate::rebuild;

/// Re-run AST extraction + build + optional cluster + report for code
/// files.
///
/// Acquires a per-repo advisory lock (unless `opts.acquire_lock` is `false`).
/// Returns `Ok(true)` when outputs were updated, `Ok(false)` when the
/// rebuild was skipped (lock held, no tracked files changed, shrink
/// guard refused).
///
/// Ports `_rebuild_code` from Python.
///
/// # Errors
///
/// Returns [`WatchError`] on pipeline failure.
pub fn rebuild_code(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    opts: rebuild::RebuildOptions,
) -> Result<bool, WatchError> {
    rebuild::rebuild_code(watch_path, changed_paths, opts)
}

/// Returns `true` if any of the paths has an extension that is **not** in
/// [`CODE_EXTENSIONS`] (i.e. a doc, paper, or image file requiring LLM re-extraction).
#[must_use]
fn has_non_code(changed_paths: &[PathBuf]) -> bool {
    changed_paths.iter().any(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_none_or(|e| !CODE_EXTENSIONS.contains(&e))
    })
}

/// Returns `true` if any of the paths has an extension in [`CODE_EXTENSIONS`]
/// (i.e. a source file that can be rebuilt without LLM re-extraction).
#[must_use]
fn has_code(changed_paths: &[PathBuf]) -> bool {
    changed_paths.iter().any(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| CODE_EXTENSIONS.contains(&e))
    })
}

/// Watch `watch_path` for file changes and auto-update the graph.
///
/// - Code-only changes: calls [`rebuild_code`].
/// - Doc/paper/image changes: calls [`notify_only`].
///
/// Uses `notify-debouncer-full` to batch rapid saves over `debounce`
/// seconds. Blocks until interrupted (Ctrl-C / `SIGINT`).
///
/// Ports `watch` from Python (which used `watchdog`).
///
/// # Errors
///
/// Returns [`WatchError::Notify`] if the underlying watcher cannot be
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

        let changed: Vec<PathBuf> = events
            .into_iter()
            .flat_map(|ev| ev.event.paths)
            .filter(|p| {
                if p.is_dir() {
                    return false;
                }
                if !ignore_patterns.is_empty() && is_ignored(p, &watch_root, &ignore_patterns) {
                    return false;
                }
                let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
                    return false;
                };
                if !WATCHED_EXTENSIONS.contains(&ext) {
                    return false;
                }
                if p.components()
                    .any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.')))
                {
                    return false;
                }
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
            let opts = rebuild::RebuildOptions {
                lock: rebuild::LockPolicy::TryAcquire,
                ..rebuild::RebuildOptions::default()
            };
            match rebuild_code(watch_path, Some(&changed), opts) {
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
