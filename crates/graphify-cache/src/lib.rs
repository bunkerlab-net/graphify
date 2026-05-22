//! Per-file extraction cache. Ports `graphify-py/graphify/cache.py`.
//!
//! Cache layout under `<root>/graphify-out/cache/`:
//! - `ast/<hash>.json` — AST extraction results
//! - `semantic/<hash>.json` — LLM/semantic extraction results
//! - `stat-index.json` — file stat fastpath
//!
//! `GRAPHIFY_OUT` env var overrides the output dir name (relative or absolute).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Cache-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("file_hash requires a file, got: {0}")]
    NotAFile(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Stat-fastpath entry: `(size, mtime_ns) -> hash`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StatEntry {
    size: u64,
    mtime_ns: u128,
    hash: String,
}

/// Process-wide stat index, lazily initialised per root.
static STAT_INDEX: LazyLock<Mutex<StatIndexState>> =
    LazyLock::new(|| Mutex::new(StatIndexState::default()));

#[derive(Default)]
struct StatIndexState {
    entries: IndexMap<String, StatEntry>,
    root: Option<PathBuf>,
    dirty: bool,
}

fn lock_index() -> std::sync::MutexGuard<'static, StatIndexState> {
    #[allow(clippy::expect_used)] // mutex poisoning here is unrecoverable; surface the panic loudly
    STAT_INDEX.lock().expect("STAT_INDEX mutex poisoned")
}

fn graphify_out() -> String {
    std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_string())
}

fn out_base(root: &Path) -> PathBuf {
    let out = PathBuf::from(graphify_out());
    if out.is_absolute() {
        out
    } else {
        let resolved = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        resolved.join(out)
    }
}

fn stat_index_file(root: &Path) -> PathBuf {
    out_base(root).join("cache").join("stat-index.json")
}

fn ensure_stat_index(root: &Path) {
    let mut state = lock_index();
    if state.root.is_some() {
        return;
    }
    let root_resolved = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    state.root = Some(root_resolved.clone());
    let path = stat_index_file(&root_resolved);
    if let Ok(text) = fs::read_to_string(&path)
        && let Ok(parsed) = serde_json::from_str::<IndexMap<String, StatEntry>>(&text)
    {
        state.entries = parsed;
    }
}

/// Flush the in-memory stat index to disk if dirty. Call before process exit
/// when running outside the test harness.
///
/// # Errors
///
/// Returns `CacheError::Io` if the index file or its parent directory cannot
/// be written, or `CacheError::Json` if serialisation fails.
pub fn flush_stat_index() -> Result<(), CacheError> {
    let mut state = lock_index();
    if !state.dirty {
        return Ok(());
    }
    let Some(root) = state.root.clone() else {
        return Ok(());
    };
    let path = stat_index_file(&root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
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

/// Reset the global stat index. Test-only; not part of the public contract.
#[doc(hidden)]
pub fn _reset_stat_index_for_tests() {
    let mut state = lock_index();
    state.entries.clear();
    state.root = None;
    state.dirty = false;
}

/// Strip YAML frontmatter from Markdown content, returning only the body.
#[must_use]
pub fn body_content(content: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(content);
    if let Some(after_open) = text.strip_prefix("---")
        && let Some(end) = after_open.find("\n---")
    {
        // Python uses `text.find("\n---", 3)` returning the absolute index of
        // the newline before the closing ---. `text[end+4:]` slices past
        // "\n---" (4 chars). Replicate that exactly so byte-identity holds.
        let body_start = end + "\n---".len();
        return after_open.as_bytes()[body_start..].to_vec();
    }
    content.to_vec()
}

fn normalize_path(path: &Path) -> PathBuf {
    // Python's _normalize_path only does work on Windows. On Unix we return the
    // path unchanged.
    if cfg!(windows) {
        let s = path.to_string_lossy();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        PathBuf::from(s.to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// SHA256 of file contents + path relative to root.
///
/// # Errors
///
/// Returns `CacheError::NotAFile` if `path` is not a regular file, or
/// `CacheError::Io` on read failure.
pub fn file_hash<P: AsRef<Path>>(path: P, root: &Path) -> Result<String, CacheError> {
    let p = normalize_path(path.as_ref());
    let root = normalize_path(root);
    if !p.is_file() {
        return Err(CacheError::NotAFile(p));
    }

    ensure_stat_index(&root);
    let abs_key = p.canonicalize().unwrap_or_else(|_| p.clone());
    let abs_key_str = abs_key.to_string_lossy().to_string();

    let meta = p.metadata()?;
    let size = meta.len();
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or_default();

    {
        let state = lock_index();
        if let Some(entry) = state.entries.get(&abs_key_str)
            && entry.size == size
            && entry.mtime_ns == mtime_ns
        {
            return Ok(entry.hash.clone());
        }
    }

    let raw = fs::read(&p)?;
    let content = if p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
    {
        body_content(&raw)
    } else {
        raw
    };
    let mut hasher = Sha256::new();
    hasher.update(&content);
    hasher.update([0u8]);
    let root_resolved = root.canonicalize().unwrap_or_else(|_| root.clone());
    let path_for_hash = match abs_key.strip_prefix(&root_resolved) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => abs_key.clone(),
    };
    let posix = posix_string(&path_for_hash).to_lowercase();
    hasher.update(posix.as_bytes());
    let digest = hex::encode(hasher.finalize());

    {
        let mut state = lock_index();
        state.entries.insert(
            abs_key_str,
            StatEntry {
                size,
                mtime_ns,
                hash: digest.clone(),
            },
        );
        state.dirty = true;
    }

    Ok(digest)
}

fn posix_string(path: &Path) -> String {
    let mut out = String::new();
    let mut first = true;
    for comp in path.components() {
        use std::path::Component::{CurDir, Normal, ParentDir, Prefix, RootDir};
        match comp {
            Prefix(_) | RootDir => {
                out.push('/');
                first = false;
            }
            CurDir => {}
            ParentDir => {
                if !first {
                    out.push('/');
                }
                out.push_str("..");
                first = false;
            }
            Normal(n) => {
                if !first && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(&n.to_string_lossy());
                first = false;
            }
        }
    }
    out
}

/// Return `graphify-out/cache/{kind}/`, creating it if needed.
///
/// # Errors
///
/// Returns `CacheError::Io` if the directory could not be created.
pub fn cache_dir(root: &Path, kind: &str) -> Result<PathBuf, CacheError> {
    let d = out_base(root).join("cache").join(kind);
    fs::create_dir_all(&d)?;
    Ok(d)
}

/// Return the cached extraction result for `path` if the hash matches.
#[must_use]
pub fn load_cached(path: &Path, root: &Path, kind: &str) -> Option<Value> {
    let hash = file_hash(path, root).ok()?;
    let dir = cache_dir(root, kind).ok()?;
    let entry = dir.join(format!("{hash}.json"));
    if let Ok(text) = fs::read_to_string(&entry) {
        if let Ok(v) = serde_json::from_str(&text) {
            return Some(v);
        }
        return None;
    }
    // Legacy flat-cache fallback for AST entries.
    if kind == "ast" {
        let legacy = out_base(root).join("cache").join(format!("{hash}.json"));
        if let Ok(text) = fs::read_to_string(&legacy) {
            return serde_json::from_str(&text).ok();
        }
    }
    None
}

/// Save an extraction result for `path`.
///
/// # Errors
///
/// Returns `CacheError::Io` on filesystem failure, `CacheError::Json` on
/// serialisation failure, or any error from [`file_hash`].
pub fn save_cached(path: &Path, result: &Value, root: &Path, kind: &str) -> Result<(), CacheError> {
    if !path.is_file() {
        return Ok(());
    }
    let hash = file_hash(path, root)?;
    let dir = cache_dir(root, kind)?;
    let entry = dir.join(format!("{hash}.json"));
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!("{hash}."))
        .suffix(".tmp")
        .tempfile_in(&dir)?;
    let serialized = serde_json::to_vec(result)?;
    tmp.write_all(&serialized)?;
    tmp.flush()?;
    tmp.persist(&entry).map_err(|e| CacheError::Io(e.error))?;
    Ok(())
}

/// Return the set of file hashes that have a cache entry (any kind).
#[must_use]
pub fn cached_files(root: &Path) -> std::collections::BTreeSet<String> {
    let mut hashes = std::collections::BTreeSet::new();
    let base = out_base(root).join("cache");

    if let Ok(dir) = fs::read_dir(&base) {
        for entry in dir.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
            {
                hashes.insert(stem.to_string());
            }
        }
    }
    for kind in ["ast", "semantic"] {
        let d = base.join(kind);
        if let Ok(dir) = fs::read_dir(&d) {
            for entry in dir.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json")
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                {
                    hashes.insert(stem.to_string());
                }
            }
        }
    }
    hashes
}

/// Delete all cache entries.
///
/// # Errors
///
/// Returns `CacheError::Io` on filesystem failure.
pub fn clear_cache(root: &Path) -> Result<(), CacheError> {
    let base = out_base(root).join("cache");
    if let Ok(dir) = fs::read_dir(&base) {
        for entry in dir.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                fs::remove_file(&p)?;
            }
        }
    }
    for kind in ["ast", "semantic"] {
        let d = base.join(kind);
        if let Ok(dir) = fs::read_dir(&d) {
            for entry in dir.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json") {
                    fs::remove_file(&p)?;
                }
            }
        }
    }
    Ok(())
}

/// Output of [`check_semantic_cache`]: separated cached + uncached lists.
#[derive(Debug, Default)]
pub struct SemanticCacheSplit {
    pub cached_nodes: Vec<Value>,
    pub cached_edges: Vec<Value>,
    pub cached_hyperedges: Vec<Value>,
    pub uncached_files: Vec<String>,
}

/// Check semantic extraction cache for a list of file paths.
#[must_use]
pub fn check_semantic_cache(files: &[String], root: &Path) -> SemanticCacheSplit {
    let mut split = SemanticCacheSplit::default();
    for fpath in files {
        let mut p = PathBuf::from(fpath);
        if !p.is_absolute() {
            p = root.join(&p);
        }
        if let Some(Value::Object(map)) = load_cached(&p, root, "semantic") {
            if let Some(Value::Array(ns)) = map.get("nodes") {
                split.cached_nodes.extend(ns.iter().cloned());
            }
            if let Some(Value::Array(es)) = map.get("edges") {
                split.cached_edges.extend(es.iter().cloned());
            }
            if let Some(Value::Array(hs)) = map.get("hyperedges") {
                split.cached_hyperedges.extend(hs.iter().cloned());
            }
        } else {
            split.uncached_files.push(fpath.clone());
        }
    }
    split
}

/// Save semantic extraction results to the cache, keyed by `source_file`.
///
/// Returns the number of source files cached.
///
/// # Errors
///
/// Returns `CacheError::Io` on filesystem failure or `CacheError::Json` on
/// serialisation failure.
pub fn save_semantic_cache(
    nodes: &[Value],
    edges: &[Value],
    hyperedges: &[Value],
    root: &Path,
) -> Result<usize, CacheError> {
    type SemanticBuckets = (Vec<Value>, Vec<Value>, Vec<Value>);
    let mut by_file: IndexMap<String, SemanticBuckets> = IndexMap::new();
    for n in nodes {
        if let Some(src) = n.get("source_file").and_then(Value::as_str)
            && !src.is_empty()
        {
            by_file
                .entry(src.to_string())
                .or_default()
                .0
                .push(n.clone());
        }
    }
    for e in edges {
        if let Some(src) = e.get("source_file").and_then(Value::as_str)
            && !src.is_empty()
        {
            by_file
                .entry(src.to_string())
                .or_default()
                .1
                .push(e.clone());
        }
    }
    for h in hyperedges {
        if let Some(src) = h.get("source_file").and_then(Value::as_str)
            && !src.is_empty()
        {
            by_file
                .entry(src.to_string())
                .or_default()
                .2
                .push(h.clone());
        }
    }

    let mut saved = 0;
    for (fpath, (n, e, h)) in &by_file {
        let mut p = PathBuf::from(fpath);
        if !p.is_absolute() {
            p = root.join(&p);
        }
        if p.is_file() {
            let payload = serde_json::json!({
                "nodes": n,
                "edges": e,
                "hyperedges": h,
            });
            save_cached(&p, &payload, root, "semantic")?;
            saved += 1;
        }
    }
    Ok(saved)
}
