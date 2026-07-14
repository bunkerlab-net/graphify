//! Process-wide stat index used as the file-hash fastpath.
//!
//! The stat index maps `<absolute path> -> (size, mtime_ns, sha256_hash)`.
//! When `file_hash` is called on a path whose `(size, mtime_ns)` matches
//! the cached entry, we skip rehashing entirely.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};

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

/// Process-wide stat index. Each cache-file root's on-disk index is loaded
/// once and kept under its resolved key, so two invocations with different
/// roots (e.g. distinct `--out` dirs in one process) never share — and thus
/// never clobber — one in-memory index.
static STAT_INDEX: LazyLock<Mutex<StatIndex>> = LazyLock::new(|| Mutex::new(StatIndex::default()));

#[derive(Default)]
pub(crate) struct StatIndex {
    /// Per-root states keyed by the resolved cache-file root.
    pub(crate) roots: HashMap<PathBuf, RootState>,
}

/// One cache-file root's in-memory index.
#[derive(Default)]
pub(crate) struct RootState {
    pub(crate) entries: IndexMap<String, StatEntry>,
    pub(crate) dirty: bool,
    /// Whether this root's on-disk index has been loaded yet.
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

/// Resolve the cache-file root and lazily load its on-disk index, returning
/// the resolved root key.
///
/// Callers thread the returned key through their per-root lookups so distinct
/// roots each read and write their OWN index rather than the first-seen one.
/// The index only determines the cache FILE location (entry keys are absolute
/// paths), so honouring an explicit `cache_root` keeps `detect()`'s word-count
/// cache under the requested `--out` dir instead of polluting the scanned
/// corpus with a stray graphify-out/ (#1747).
pub(crate) fn ensure_stat_index(root: &Path, cache_root: Option<&Path>) -> PathBuf {
    let base = cache_root.unwrap_or(root);
    let key = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let mut index = lock_index();
    let state = index.roots.entry(key.clone()).or_default();
    if !state.loaded {
        state.loaded = true;
        let path = stat_index_file(&key);
        if let Ok(text) = fs::read_to_string(&path)
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
    for (root, state) in &mut index.roots {
        if !state.dirty {
            continue;
        }
        let path = stat_index_file(root);
        let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent_dir)?;
        let mut tmp = tempfile::Builder::new()
            .prefix("stat-index.")
            .suffix(".tmp")
            .tempfile_in(parent_dir)?;
        let serialized = serde_json::to_vec(&state.entries)?;
        tmp.write_all(&serialized)?;
        tmp.flush()?;
        tmp.persist(&path).map_err(|e| CacheError::Io(e.error))?;
        state.dirty = false;
    }
    Ok(())
}

/// RAII sentinel that flushes the stat index on `drop`.
///
/// Used with `OnceLock` so registration is idempotent: calling
/// [`ensure_atexit_flush_registered`] multiple times is safe.
///
/// **Trade-off**: `Drop` runs when Rust's normal stack unwinding happens
/// (e.g. on `panic="unwind"`). It does NOT run on `std::process::exit()`,
/// SIGKILL, or `panic="abort"`. For the common case (graceful shutdown)
/// this is sufficient to persist the stat index.
struct FlushSentinel;

impl Drop for FlushSentinel {
    /// Flush the stat index to disk on drop, discarding any error
    /// silently — the cache is a performance optimisation, not critical
    /// data.
    fn drop(&mut self) {
        let _ = flush_stat_index();
    }
}

static ATEXIT: OnceLock<FlushSentinel> = OnceLock::new();

/// Register a process-exit flush of the stat index.
///
/// Idempotent — safe to call more than once. The flush is best-effort;
/// errors are silently discarded.
///
/// # Limitations
///
/// Does NOT flush on `std::process::exit()`, SIGKILL, or `panic="abort"`.
pub fn ensure_atexit_flush_registered() {
    ATEXIT.get_or_init(|| FlushSentinel);
}

/// Reset the global stat index. Test-only; not part of the public
/// contract.
#[doc(hidden)]
pub fn _reset_stat_index_for_tests() {
    lock_index().roots.clear();
}
