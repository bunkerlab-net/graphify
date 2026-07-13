//! Process-wide stat index used as the file-hash fastpath.
//!
//! The stat index maps `<absolute path> -> (size, mtime_ns, sha256_hash)`.
//! When `file_hash` is called on a path whose `(size, mtime_ns)` matches
//! the cached entry, we skip rehashing entirely.

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

/// Process-wide stat index, lazily initialised per root.
static STAT_INDEX: LazyLock<Mutex<StatIndexState>> =
    LazyLock::new(|| Mutex::new(StatIndexState::default()));

#[derive(Default)]
pub(crate) struct StatIndexState {
    pub(crate) entries: IndexMap<String, StatEntry>,
    pub(crate) root: Option<PathBuf>,
    pub(crate) dirty: bool,
}

/// Acquire the global stat-index mutex, panicking if it is poisoned.
///
/// Mutex poisoning here is unrecoverable: it indicates a previous panic
/// while the index was being mutated, so the index state may be torn.
pub(crate) fn lock_index() -> std::sync::MutexGuard<'static, StatIndexState> {
    #[allow(clippy::expect_used)] // mutex poisoning here is unrecoverable; surface the panic loudly
    STAT_INDEX.lock().expect("STAT_INDEX mutex poisoned")
}

/// Load the stat index from disk into the global state if it has not
/// already been initialised for `root`.
///
/// The first call to `file_hash` per process loads the index; subsequent
/// calls hit the in-memory state.
///
/// **Single-root per process**: `state.root` is set on the first call and
/// later calls with a different `root` are silently ignored. The index
/// file at `stat_index_file(root)` is loaded only once, so callers must
/// not expect per-call rooting.
pub(crate) fn ensure_stat_index(root: &Path, cache_root: Option<&Path>) {
    let mut state = lock_index();
    if state.root.is_some() {
        return;
    }
    // The stat index only determines the cache FILE location (entry keys are
    // absolute paths), so honouring an explicit `cache_root` keeps detect()'s
    // word-count cache under the requested `--out` dir instead of polluting the
    // scanned corpus with a stray graphify-out/ (#1747).
    let base = cache_root.unwrap_or(root);
    let root_resolved = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    state.root = Some(root_resolved.clone());
    let path = stat_index_file(&root_resolved);
    if let Ok(text) = fs::read_to_string(&path)
        && let Ok(parsed) = serde_json::from_str::<IndexMap<String, StatEntry>>(&text)
    {
        state.entries = parsed;
    }
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
    let mut state = lock_index();
    if !state.dirty {
        return Ok(());
    }
    let Some(root) = state.root.clone() else {
        return Ok(());
    };
    let path = stat_index_file(&root);
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
    let mut state = lock_index();
    state.entries.clear();
    state.root = None;
    state.dirty = false;
}
