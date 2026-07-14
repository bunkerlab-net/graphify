//! Process-wide stat index used as the file-hash fastpath.
//!
//! The stat index maps `<absolute path> -> (size, mtime_ns, sha256_hash)`.
//! When `file_hash` is called on a path whose `(size, mtime_ns)` matches
//! the cached entry, we skip rehashing entirely.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::CacheError;
use crate::paths::stat_index_file;

/// Stat-fastpath entry: `(size, mtime_ns)` plus an optional cached content hash
/// and an optional cached word count. Either payload may be absent — a
/// word-count-only entry (from [`crate::cached_word_count`]) carries no `hash`,
/// and a hash-only entry carries no `word_count` — so both are `Option` and the
/// `file_hash` fastpath requires `hash` present. Mirrors graphify-py's shared
/// stat index (#1656).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StatEntry {
    pub(crate) size: u64,
    pub(crate) mtime_ns: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) word_count: Option<u64>,
}

/// Process-wide stat index. Each on-disk index is loaded once and kept under
/// its resolved FILE path, so two invocations with different cache roots that
/// resolve to different index files never share one in-memory state — while
/// two that resolve to the SAME file (an absolute `GRAPHIFY_OUT` override)
/// correctly share it instead of clobbering each other.
static STAT_INDEX: LazyLock<Mutex<StatIndex>> = LazyLock::new(|| Mutex::new(StatIndex::default()));

#[derive(Default)]
pub(crate) struct StatIndex {
    /// Per-index states keyed by the resolved `stat-index.json` file path.
    pub(crate) roots: HashMap<PathBuf, RootState>,
}

/// One cache-file root's in-memory index.
#[derive(Default)]
pub(crate) struct RootState {
    pub(crate) entries: IndexMap<String, StatEntry>,
    pub(crate) dirty: bool,
    /// Whether this index's on-disk file has been loaded yet.
    loaded: bool,
}

/// Acquire the global stat-index mutex, panicking if it is poisoned.
///
/// Mutex poisoning here is unrecoverable: it indicates a previous panic
/// while the index was being mutated, so the index state may be torn.
pub(crate) fn lock_index() -> std::sync::MutexGuard<'static, StatIndex> {
    #[allow(clippy::expect_used)] // mutex poisoning here is unrecoverable; surface the panic loudly
    STAT_INDEX.lock().expect("STAT_INDEX mutex poisoned")
}

/// Resolve the stat-index file for `root`/`cache_root` and lazily load it,
/// returning that file path as the state key.
///
/// Callers thread the returned key through their per-index lookups. The key is
/// the resolved index FILE path (not the cache root): honouring an explicit
/// `cache_root` keeps `detect()`'s word-count cache under the requested `--out`
/// dir rather than polluting the scanned corpus (#1747), and keying by the file
/// means an absolute `GRAPHIFY_OUT` — where `out_base` ignores the root and every
/// root maps to one file — shares a single state instead of competing.
pub(crate) fn ensure_stat_index(root: &Path, cache_root: Option<&Path>) -> PathBuf {
    let base = cache_root.unwrap_or(root);
    let base_resolved = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let key = stat_index_file(&base_resolved);
    let mut index = lock_index();
    let state = index.roots.entry(key.clone()).or_default();
    if !state.loaded {
        state.loaded = true;
        if let Ok(text) = fs::read_to_string(&key)
            && let Ok(parsed) = serde_json::from_str::<IndexMap<String, StatEntry>>(&text)
        {
            state.entries = parsed;
        }
    }
    key
}

/// Flush the in-memory stat index to disk if dirty.
///
/// Call before process exit when running outside the test harness.
/// Best-effort: if the index has not been initialised, this is a no-op.
///
/// # Errors
///
/// Returns [`CacheError::Io`] if the index file or its parent directory
/// cannot be written, or [`CacheError::Json`] if serialisation fails.
pub fn flush_stat_index() -> Result<(), CacheError> {
    let mut index = lock_index();
    for (path, state) in &mut index.roots {
        if !state.dirty {
            continue;
        }
        let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent_dir)?;
        let mut tmp = tempfile::Builder::new()
            .prefix("stat-index.")
            .suffix(".tmp")
            .tempfile_in(parent_dir)?;
        let serialized = serde_json::to_vec(&state.entries)?;
        tmp.write_all(&serialized)?;
        tmp.flush()?;
        tmp.persist(path.as_path())
            .map_err(|e| CacheError::Io(e.error))?;
        state.dirty = false;
    }
    Ok(())
}

/// RAII guard that flushes the stat index to disk when dropped.
///
/// Bind it for the lifetime of the process's work (e.g. in the CLI `run`); it
/// persists the index on a normal return, an error return, or a `panic="unwind"`,
/// mirroring graphify-py's `atexit`-registered flush.
///
/// A `static` guard does NOT work here: Rust never drops `static` items at
/// process exit, so the flush must be owned by a live stack frame.
///
/// **Trade-off**: `Drop` does NOT run on `std::process::exit()`, SIGKILL, or
/// `panic="abort"`. For graceful shutdown this suffices to persist the index.
#[must_use = "the stat index is flushed when this guard drops; bind it for the process's lifetime"]
pub struct StatIndexFlushGuard(());

impl StatIndexFlushGuard {
    /// Create a guard that flushes the stat index when it goes out of scope.
    pub fn new() -> Self {
        Self(())
    }
}

impl Default for StatIndexFlushGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StatIndexFlushGuard {
    /// Flush the stat index to disk, discarding any error silently — the cache
    /// is a performance optimisation, not critical data.
    fn drop(&mut self) {
        let _ = flush_stat_index();
    }
}

/// Reset the global stat index. Test-only; not part of the public
/// contract.
#[doc(hidden)]
pub fn _reset_stat_index_for_tests() {
    lock_index().roots.clear();
}
