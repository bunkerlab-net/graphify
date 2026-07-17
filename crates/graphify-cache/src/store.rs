//! On-disk cache: load / save individual entries, list all entries, clear.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::CacheError;
use crate::hash::file_hash;
use crate::paths::{EXTRACTOR_VERSION, cache_dir_versioned, out_base, semantic_cache_dirs};

/// Return the cached extraction result for `path` if its hash matches.
///
/// `kind` is the cache namespace (`"ast"` or `"semantic"`). AST entries are
/// read from the per-version subdirectory only — entries written by other
/// graphify versions (the legacy flat layout and the unversioned `cache/ast/`
/// layout) are deliberately not consulted, since they were produced by a
/// different extractor and may be stale.
///
/// Relative `source_file` fields are re-anchored against `root` so callers see
/// the same absolute-path shape a fresh in-process extraction would produce
/// (#777). Returns `None` if no matching entry exists or it fails to parse.
/// `root` anchors the content-hash key and `source_file` relativization;
/// `cache_root` (when `Some`) decouples *where* the cache directory lives from
/// that anchor, so the cache never lands inside a read-only/analysed source
/// tree (#1774). `None` falls back to `root`.
#[must_use]
pub fn load_cached(
    path: &Path,
    root: &Path,
    kind: &str,
    cache_root: Option<&Path>,
) -> Option<Value> {
    load_cached_versioned(path, root, kind, EXTRACTOR_VERSION, cache_root)
}

/// Like [`load_cached`] but with the AST namespace version supplied
/// explicitly (used by tests to simulate an upgrade).
#[must_use]
pub fn load_cached_versioned(
    path: &Path,
    root: &Path,
    kind: &str,
    version: &str,
    cache_root: Option<&Path>,
) -> Option<Value> {
    let hash = file_hash(path, root, cache_root).ok()?;
    let location = cache_root.unwrap_or(root);
    let dir = cache_dir_versioned(location, kind, version).ok()?;
    let entry = dir.join(format!("{hash}.json"));
    let text = fs::read_to_string(&entry).ok()?;
    let mut value: Value = serde_json::from_str(&text).ok()?;
    absolutize_source_files_in(&mut value, root);
    Some(value)
}

/// Save an extraction result for `path` into the `kind` namespace.
///
/// Atomic: writes to a tempfile in the same directory, then persists via
/// rename so concurrent readers never see a half-written file. No-op for
/// paths that are not regular files.
///
/// Absolute `source_file` fields are relativized against `root` before write
/// so the on-disk file is portable across machines and checkout directories
/// (#777). A relativized *copy* is serialized — the caller's value keeps its
/// original absolute `source_file` form, which downstream extraction steps
/// (AST prefix remap) depend on.
///
/// # Errors
///
/// Returns [`CacheError::Io`] on filesystem failure, [`CacheError::Json`]
/// on serialisation failure, or any error from [`file_hash`].
/// `root` anchors the content-hash key and `source_file` relativization;
/// `cache_root` (when `Some`) is where the cache directory is written, decoupled
/// from `root` so the cache never lands inside the analysed source tree (#1774).
pub fn save_cached(
    path: &Path,
    result: &Value,
    root: &Path,
    kind: &str,
    cache_root: Option<&Path>,
) -> Result<(), CacheError> {
    save_cached_versioned(path, result, root, kind, EXTRACTOR_VERSION, cache_root)
}

/// Like [`save_cached`] but with the AST namespace version supplied
/// explicitly (used by tests to simulate an upgrade).
///
/// # Errors
///
/// See [`save_cached`].
pub fn save_cached_versioned(
    path: &Path,
    result: &Value,
    root: &Path,
    kind: &str,
    version: &str,
    cache_root: Option<&Path>,
) -> Result<(), CacheError> {
    if !path.is_file() {
        return Ok(());
    }
    // Serialize a relativized copy rather than mutating the caller's value.
    let on_disk: Cow<'_, Value> = if has_path_buckets(result) {
        let mut copy = result.clone();
        relativize_source_files_in(&mut copy, root);
        Cow::Owned(copy)
    } else {
        Cow::Borrowed(result)
    };

    let hash = file_hash(path, root, cache_root)?;
    let location = cache_root.unwrap_or(root);
    let dir = cache_dir_versioned(location, kind, version)?;
    let entry = dir.join(format!("{hash}.json"));
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!("{hash}."))
        .suffix(".tmp")
        .tempfile_in(&dir)?;
    let serialized = serde_json::to_vec(on_disk.as_ref())?;
    tmp.write_all(&serialized)?;
    tmp.flush()?;
    tmp.persist(&entry).map_err(|e| CacheError::Io(e.error))?;
    Ok(())
}

/// Return the set of file hashes that have at least one cache entry
/// (legacy flat layout + `ast/` recursively, covering per-version subdirs,
/// + every `semantic*` namespace).
#[must_use]
pub fn cached_files(root: &Path) -> BTreeSet<String> {
    let mut hashes = BTreeSet::new();
    let base = out_base(root).join("cache");

    // Legacy flat entries directly under cache/.
    collect_json_stems(&base, false, &mut hashes);
    // AST entries recurse into per-version subdirs.
    collect_json_stems(&base.join("ast"), true, &mut hashes);
    // Every semantic namespace (`semantic/`, `semantic-deep/`, and any future
    // `semantic-<mode>/`), enumerated from disk (#1894).
    for dir in semantic_cache_dirs(root) {
        collect_json_stems(&dir, false, &mut hashes);
    }
    hashes
}

/// Insert the file stems of `*.json` entries under `dir` into `out`,
/// recursing into subdirectories when `recursive` is set.
fn collect_json_stems(dir: &Path, recursive: bool, out: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if recursive {
                collect_json_stems(&p, true, out);
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("json")
            && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
        {
            out.insert(stem.to_string());
        }
    }
}

/// Delete every cache entry under `<root>/graphify-out/cache/`.
///
/// Includes the legacy flat layout, the `ast/` tree (recursively, covering
/// per-version subdirectories), and every `semantic*` namespace.
///
/// # Errors
///
/// Returns [`CacheError::Io`] on filesystem failure.
pub fn clear_cache(root: &Path) -> Result<(), CacheError> {
    let base = out_base(root).join("cache");
    remove_json_files(&base, false)?;
    remove_json_files(&base.join("ast"), true)?;
    for dir in semantic_cache_dirs(root) {
        remove_json_files(&dir, false)?;
    }
    Ok(())
}

/// Remove `*.json` files under `dir`, recursing into subdirectories when
/// `recursive` is set. Directories themselves are left in place (mirrors
/// Python's `glob(...).unlink()`).
fn remove_json_files(dir: &Path, recursive: bool) -> Result<(), CacheError> {
    // Refuse to traverse a symlinked cache directory: following it could delete
    // files outside the cache tree. A check-then-use guard (the workspace's
    // `path_guard` model), consistent with the Obsidian/hook symlink guards.
    if is_symlink(dir) {
        return Err(symlink_err(dir));
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let p = entry.path();
        // `is_dir()` follows symlinks, so check the link itself first: a
        // symlinked subdir could redirect deletion outside the cache tree.
        if is_symlink(&p) {
            if p.is_dir() {
                return Err(symlink_err(&p));
            }
            // A symlinked file: skip it — never follow the link to its target.
            continue;
        }
        if p.is_dir() {
            if recursive {
                remove_json_files(&p, true)?;
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("json") {
            fs::remove_file(&p)?;
        }
    }
    Ok(())
}

/// True when `p` is a symlink (does not follow it).
fn is_symlink(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

/// A [`CacheError`] refusing to traverse a symlinked cache directory.
fn symlink_err(p: &Path) -> CacheError {
    CacheError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "{} is a symlink; refusing to traverse the cache through it",
            p.display()
        ),
    ))
}

/// `true` if `value` is an object with a truthy `nodes`, `edges`, or
/// `hyperedges` bucket — i.e. an extraction fragment worth relativizing.
fn has_path_buckets(value: &Value) -> bool {
    value.as_object().is_some_and(|o| {
        ["nodes", "edges", "hyperedges", "raw_calls"]
            .iter()
            .any(|k| o.get(*k).is_some_and(json_truthy))
    })
}

/// Python-style truthiness for the bucket-presence check.
fn json_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_none_or(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Rewrite absolute `source_file` fields in `payload` as forward-slash paths
/// relative to `root` (#777). Only `root` is resolved — `source_file` itself
/// is relativized symbolically so in-root symlinks keep their own name rather
/// than the resolved target. Out-of-root and already-relative paths are left
/// unchanged.
fn relativize_source_files_in(payload: &mut Value, root: &Path) {
    let Ok(root_resolved) = root.canonicalize() else {
        return;
    };
    for_each_source_file(payload, |source| {
        let sp = PathBuf::from(source);
        if !sp.is_absolute() {
            return None;
        }
        // strip_prefix on the *unresolved* source: under-root paths become
        // relative, anything that would need `..` to reach (escaped root)
        // stays absolute — matching the Python relpath + `..` guard.
        sp.strip_prefix(&root_resolved)
            .ok()
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
    });
}

/// Inverse of [`relativize_source_files_in`]: re-anchor relative
/// `source_file` fields against `root`. Absolute values pass through (legacy
/// cache entries).
fn absolutize_source_files_in(payload: &mut Value, root: &Path) {
    let Ok(root_resolved) = root.canonicalize() else {
        return;
    };
    for_each_source_file(payload, |source| {
        let sp = PathBuf::from(source);
        if sp.is_absolute() {
            return None;
        }
        Some(root_resolved.join(&sp).to_string_lossy().into_owned())
    });
}

/// Apply `rewrite` to every string `source_file` in the `nodes`, `edges`,
/// `hyperedges`, and `raw_calls` buckets of `payload`. A returned `Some(new)`
/// replaces the field; `None` leaves it untouched.
///
/// `raw_calls` carries `source_file` the same way (#1739 Pascal/Delphi cross-file
/// inherited-call resolution), so it needs the same portable-path treatment for
/// cache entries to round-trip across machines / checkout directories.
fn for_each_source_file(payload: &mut Value, mut rewrite: impl FnMut(&str) -> Option<String>) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    for bucket in ["nodes", "edges", "hyperedges", "raw_calls"] {
        let Some(Value::Array(items)) = obj.get_mut(bucket) else {
            continue;
        };
        for item in items.iter_mut() {
            let Some(map) = item.as_object_mut() else {
                continue;
            };
            let Some(Value::String(source)) = map.get("source_file") else {
                continue;
            };
            if source.is_empty() {
                continue;
            }
            if let Some(new) = rewrite(source) {
                map.insert("source_file".to_string(), Value::String(new));
            }
        }
    }
}
