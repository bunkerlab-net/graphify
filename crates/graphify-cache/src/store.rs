//! On-disk cache: load / save individual entries, list all entries, clear.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use crate::error::CacheError;
use crate::hash::file_hash;
use crate::paths::{cache_dir, out_base};

/// Return the cached extraction result for `path` if its hash matches.
///
/// `kind` is the cache namespace (`"ast"` or `"semantic"`). Falls back to
/// the legacy flat layout (`cache/<hash>.json`) for `"ast"` so existing
/// Python-format caches remain readable.
///
/// Returns `None` if no matching entry exists.
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
    if kind == "ast" {
        let legacy = out_base(root).join("cache").join(format!("{hash}.json"));
        if let Ok(text) = fs::read_to_string(&legacy) {
            return serde_json::from_str(&text).ok();
        }
    }
    None
}

/// Save an extraction result for `path` into the `kind` namespace.
///
/// Atomic: writes to a tempfile in the same directory, then persists via
/// rename so concurrent readers never see a half-written file. No-op for
/// paths that are not regular files.
///
/// # Errors
///
/// Returns [`CacheError::Io`] on filesystem failure, [`CacheError::Json`]
/// on serialisation failure, or any error from [`file_hash`].
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

/// Return the set of file hashes that have at least one cache entry
/// (legacy flat layout + `ast/` + `semantic/`).
#[must_use]
pub fn cached_files(root: &Path) -> BTreeSet<String> {
    let mut hashes = BTreeSet::new();
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

/// Delete every cache entry under `<root>/graphify-out/cache/`.
///
/// Includes the legacy flat layout and the `ast/` / `semantic/`
/// subdirectories.
///
/// # Errors
///
/// Returns [`CacheError::Io`] on filesystem failure.
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
