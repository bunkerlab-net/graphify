//! Manifest persistence and incremental detection.
//!
//! Ports `save_manifest`, `load_manifest`, `detect_incremental`, and
//! `_md5_file` from `graphify-py/graphify/detect.py`.

use std::io::Read;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::DetectError;

/// File-count threshold above which manifest hashing is dispatched to Rayon.
/// Below this, the sequential path avoids thread-pool overhead.
const PARALLEL_HASH_THRESHOLD: usize = 16;

/// Typed result from [`detect_incremental_with_manifest`].
pub type IncrementalResult = (Vec<PathBuf>, Vec<PathBuf>, IndexMap<String, ManifestEntry>);

/// The on-disk path for the manifest, relative to the scan root.
pub const MANIFEST_PATH: &str = "graphify-out/manifest.json";

// ── Per-file manifest entry ───────────────────────────────────────────────────

/// One entry in the manifest JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// File modification time as seconds since the Unix epoch (nanosecond precision in fractional part).
    pub mtime: f64,
    /// MD5 hex digest of the file content at the time of the last AST extraction.
    pub ast_hash: String,
    /// MD5 hex digest of the file content at the time of the last semantic extraction.
    pub semantic_hash: String,
}

/// Normalises a raw JSON value from the manifest into a `ManifestEntry`, handling both legacy `{mtime, hash}` and current `{mtime, ast_hash, semantic_hash}` shapes.
fn normalise_entry(v: &serde_json::Value) -> Option<ManifestEntry> {
    match v {
        serde_json::Value::Number(n) => Some(ManifestEntry {
            mtime: n.as_f64().unwrap_or(0.0),
            ast_hash: String::new(),
            semantic_hash: String::new(),
        }),
        serde_json::Value::Object(map) => {
            let mtime = map
                .get("mtime")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            if map.contains_key("ast_hash") {
                Some(ManifestEntry {
                    mtime,
                    ast_hash: map
                        .get("ast_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    semantic_hash: map
                        .get("semantic_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            } else if let Some(h) = map.get("hash").and_then(|v| v.as_str()) {
                // Legacy {mtime, hash} → ast_hash only
                Some(ManifestEntry {
                    mtime,
                    ast_hash: h.to_string(),
                    semantic_hash: String::new(),
                })
            } else {
                Some(ManifestEntry {
                    mtime,
                    ast_hash: String::new(),
                    semantic_hash: String::new(),
                })
            }
        }
        _ => None,
    }
}

// ── MD5 hashing ──────────────────────────────────────────────────────────────

/// MD5 of file contents streamed in 64 KB chunks — for change detection only.
///
/// Returns an empty string on any I/O error.
#[must_use]
pub fn md5_file(path: &Path) -> String {
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut ctx = md5::Context::new();
    let mut buf = vec![0u8; 65536];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => ctx.consume(&buf[..n]),
            Err(_) => return String::new(),
        }
    }
    let digest = ctx.finalize();
    format!("{digest:x}")
}

/// Minimum mtime delta (1 µs) considered a real change. `f64::EPSILON`
/// (~2.2e-16) is meaningless against Unix-epoch second magnitudes and
/// causes spurious "changed" verdicts.
const MTIME_TOLERANCE: f64 = 1e-6;

// ── mtime helper ─────────────────────────────────────────────────────────────

/// Returns the file's modification time as seconds since the Unix epoch, with nanosecond precision in the fractional part.
pub(crate) fn file_mtime(path: &Path) -> Option<f64> {
    let meta = path.metadata().ok()?;
    let mtime = meta.modified().ok()?;
    let duration = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
    #[allow(clippy::cast_precision_loss)]
    // seconds fit safely in f64 mantissa for realistic timestamps
    let secs = duration.as_secs() as f64;
    #[allow(clippy::cast_precision_loss)] // subsec_nanos < 1e9, always exact in f64
    let nsecs = f64::from(duration.subsec_nanos()) / 1_000_000_000.0;
    Some(secs + nsecs)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Return `key` as a forward-slash path relative to `root` (#777).
///
/// Keys outside `root` (out-of-tree symlinked sources, external include paths)
/// and already-relative keys pass through unchanged. Only `root` is resolved —
/// the key itself is relativized symbolically so an in-root symlink is stored
/// under its own name (resolving it would point the stored entry at the symlink
/// target, which then misses on reload and re-extracts every run).
fn to_relative_for_storage(key: &str, root: &Path) -> String {
    let p = Path::new(key);
    if !p.is_absolute() {
        return key.to_string();
    }
    let Ok(root_resolved) = root.canonicalize() else {
        return key.to_string();
    };
    // `strip_prefix` on the unresolved key mirrors Python's relpath + `..`
    // guard: under-root keys become relative, escaped-root keys stay absolute.
    match p.strip_prefix(&root_resolved) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => key.to_string(),
    }
}

/// Inverse of [`to_relative_for_storage`]: re-anchor a stored `key` against
/// `root`. Already-absolute keys (legacy manifests, out-of-root entries) pass
/// through unchanged.
fn to_absolute_from_storage(key: &str, root: &Path) -> String {
    let p = Path::new(key);
    if p.is_absolute() {
        return key.to_string();
    }
    let root_resolved = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    root_resolved.join(p).to_string_lossy().into_owned()
}

/// Load the manifest from disk.
///
/// Returns an empty map on any error (missing file, parse failure, etc.).
///
/// # Errors
///
/// This function intentionally returns `Ok(empty_map)` on missing/corrupt
/// manifests (matching Python). Returns `Err` only on severe I/O problems.
pub fn load_manifest_from_path(
    manifest_path: &Path,
) -> Result<IndexMap<String, ManifestEntry>, DetectError> {
    load_manifest_from_path_with_root(manifest_path, None)
}

/// Like [`load_manifest_from_path`] but, when `root` is `Some`, re-anchors
/// stored relative keys to absolute form so callers see absolute paths
/// regardless of on-disk format (#777). Legacy absolute-keyed manifests pass
/// through unchanged.
///
/// # Errors
///
/// See [`load_manifest_from_path`].
pub fn load_manifest_from_path_with_root(
    manifest_path: &Path,
    root: Option<&Path>,
) -> Result<IndexMap<String, ManifestEntry>, DetectError> {
    let Ok(text) = std::fs::read_to_string(manifest_path) else {
        return Ok(IndexMap::new());
    };
    let raw: serde_json::Value =
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
    let Some(obj) = raw.as_object() else {
        return Ok(IndexMap::new());
    };
    let mut map: IndexMap<String, ManifestEntry> = IndexMap::new();
    for (k, v) in obj {
        if let Some(entry) = normalise_entry(v) {
            let key = match root {
                Some(r) => to_absolute_from_storage(k, r),
                None => k.clone(),
            };
            map.insert(key, entry);
        }
    }
    Ok(map)
}

/// Save the current file mtimes + content hashes for change detection.
///
/// `files` is a map of file-type → list of file paths (same structure as
/// `DetectResult::files`). `manifest_path` is the output file path.
///
/// `kind` controls which hash fields are stamped:
/// - `"ast"` — stamps `ast_hash`; preserves `semantic_hash` when unchanged.
/// - `"semantic"` — stamps `semantic_hash`; preserves `ast_hash`.
/// - `"both"` (default) — stamps both.
///
/// # Errors
///
/// Returns `DetectError::Io` on write failure.
pub fn save_manifest_to_path(
    files: &IndexMap<String, Vec<String>>,
    manifest_path: &Path,
    kind: &str,
) -> Result<(), DetectError> {
    save_manifest_to_path_with_root(files, manifest_path, kind, None)
}

/// Like [`save_manifest_to_path`] but, when `root` is `Some`, relativizes keys
/// against it before write (forward-slash, posix-style) so the on-disk manifest
/// is portable across machines and checkout locations (#777). Out-of-root
/// entries are written as absolute so they still round-trip on the saving
/// machine. When `root` is `None` the legacy absolute-keyed format is preserved.
///
/// # Errors
///
/// Returns `DetectError::Io` on write failure.
pub fn save_manifest_to_path_with_root(
    files: &IndexMap<String, Vec<String>>,
    manifest_path: &Path,
    kind: &str,
    root: Option<&Path>,
) -> Result<(), DetectError> {
    let existing = load_manifest_from_path_with_root(manifest_path, root)?;

    // Seed from existing; prune entries for deleted files.
    let mut manifest: IndexMap<String, ManifestEntry> = IndexMap::new();
    for (f, entry) in &existing {
        if Path::new(f).try_exists().unwrap_or(false) {
            manifest.insert(f.clone(), entry.clone());
        }
    }

    // Flatten the per-type file lists into a single ordered Vec so the
    // hashing pass can fan out across Rayon threads in one shot.
    let all_files: Vec<&String> = files.values().flatten().collect();

    // Hash the files (md5 + mtime stat). Per-file work is fully independent,
    // so parallelism is safe; the merging step runs sequentially below so the
    // resulting IndexMap retains insertion order.
    let hashed: Vec<(String, f64, String)> = if all_files.len() >= PARALLEL_HASH_THRESHOLD {
        all_files
            .par_iter()
            .filter_map(|f| {
                let p = Path::new(f.as_str());
                let mtime = file_mtime(p)?;
                let h = md5_file(p);
                if h.is_empty() {
                    return None;
                }
                Some(((*f).clone(), mtime, h))
            })
            .collect()
    } else {
        all_files
            .iter()
            .filter_map(|f| {
                let p = Path::new(f.as_str());
                let mtime = file_mtime(p)?;
                let h = md5_file(p);
                if h.is_empty() {
                    return None;
                }
                Some(((*f).clone(), mtime, h))
            })
            .collect()
    };

    for (f, mtime, h) in hashed {
        let prev = existing.get(&f).cloned().unwrap_or(ManifestEntry {
            mtime: 0.0,
            ast_hash: String::new(),
            semantic_hash: String::new(),
        });

        let entry = match kind {
            "ast" => ManifestEntry {
                mtime,
                ast_hash: h,
                semantic_hash: prev.semantic_hash,
            },
            "semantic" => ManifestEntry {
                mtime,
                ast_hash: prev.ast_hash,
                semantic_hash: h,
            },
            _ => ManifestEntry {
                mtime,
                ast_hash: h.clone(),
                semantic_hash: h,
            },
        };
        manifest.insert(f, entry);
    }

    // Persist in portable form when a root is given (#777): forward-slash
    // relative keys; out-of-root keys keep their absolute form.
    let manifest: IndexMap<String, ManifestEntry> = if let Some(r) = root {
        manifest
            .into_iter()
            .map(|(k, v)| (to_relative_for_storage(&k, r), v))
            .collect()
    } else {
        manifest
    };

    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent).map_err(DetectError::Io)?;
    }

    let json = serde_json::to_string_pretty(&manifest).map_err(DetectError::Json)?;
    std::fs::write(manifest_path, json).map_err(DetectError::Io)?;
    Ok(())
}

/// Run incremental detection given a previously-saved manifest.
///
/// Returns `(changed_files, deleted_files, updated_manifest)`.
///
/// `kind` controls which hash field is checked for changes:
/// - `"semantic"` (default) — re-extract when `semantic_hash` is missing or content changed.
/// - `"ast"` — re-extract when `ast_hash` is missing or content changed.
///
/// # Errors
///
/// Returns `DetectError` on I/O or parse failure.
pub fn detect_incremental_with_manifest(
    root: &Path,
    manifest_path: &Path,
    follow_symlinks: Option<bool>,
    kind: &str,
    extra_excludes: Option<&[String]>,
) -> Result<IncrementalResult, DetectError> {
    let full = crate::walk::detect(root, follow_symlinks, extra_excludes);
    // Load with `root` so a manifest written with relative keys (post-#777) is
    // re-anchored to the absolute form the rest of this function compares
    // against. Legacy absolute-keyed manifests pass through unchanged.
    let manifest = load_manifest_from_path_with_root(manifest_path, Some(root))?;

    let all_current: Vec<String> = full.files.values().flatten().cloned().collect();

    // Fan out the per-file change check across Rayon threads. Each file's
    // mtime/hash comparison is independent and dominated by I/O.
    let change_check = |f: &String| -> Option<PathBuf> {
        let p = Path::new(f);
        let stored = manifest.get(f);
        let current_mtime: f64 = file_mtime(p).unwrap_or(0.0);

        let file_changed = match stored {
            None => true,
            Some(entry) => {
                let stored_hash = if kind == "semantic" {
                    &entry.semantic_hash
                } else {
                    &entry.ast_hash
                };
                if stored_hash.is_empty() {
                    true
                } else if (current_mtime - entry.mtime).abs() > MTIME_TOLERANCE {
                    md5_file(p) != *stored_hash
                } else {
                    false
                }
            }
        };

        if file_changed {
            Some(PathBuf::from(f))
        } else {
            None
        }
    };

    let changed: Vec<PathBuf> = if all_current.len() >= PARALLEL_HASH_THRESHOLD {
        all_current.par_iter().filter_map(change_check).collect()
    } else {
        all_current.iter().filter_map(change_check).collect()
    };

    // Files in manifest that no longer exist → deleted
    let current_set: std::collections::HashSet<&str> =
        all_current.iter().map(String::as_str).collect();
    let deleted: Vec<PathBuf> = manifest
        .keys()
        .filter(|k| !current_set.contains(k.as_str()))
        .map(PathBuf::from)
        .collect();

    // Hash changed files in parallel; merge into the updated map sequentially
    // afterwards so entry insertion order matches the input.
    let rehashed: Vec<(String, f64, String)> = if changed.len() >= PARALLEL_HASH_THRESHOLD {
        changed
            .par_iter()
            .map(|f| {
                let p = f.as_path();
                let mtime = file_mtime(p).unwrap_or(0.0);
                let h = md5_file(p);
                (f.to_string_lossy().into_owned(), mtime, h)
            })
            .collect()
    } else {
        changed
            .iter()
            .map(|f| {
                let p = f.as_path();
                let mtime = file_mtime(p).unwrap_or(0.0);
                let h = md5_file(p);
                (f.to_string_lossy().into_owned(), mtime, h)
            })
            .collect()
    };

    let mut updated: IndexMap<String, ManifestEntry> = manifest.clone();
    for (key, mtime, h) in rehashed {
        let prev = updated.get(&key).cloned().unwrap_or(ManifestEntry {
            mtime: 0.0,
            ast_hash: String::new(),
            semantic_hash: String::new(),
        });
        let entry = match kind {
            "semantic" => ManifestEntry {
                mtime,
                ast_hash: prev.ast_hash,
                semantic_hash: h,
            },
            "ast" => ManifestEntry {
                mtime,
                ast_hash: h,
                semantic_hash: prev.semantic_hash,
            },
            _ => ManifestEntry {
                mtime,
                ast_hash: h.clone(),
                semantic_hash: h,
            },
        };
        updated.insert(key, entry);
    }
    // Remove deleted files from updated manifest
    for d in &deleted {
        updated.swap_remove(d.to_string_lossy().as_ref());
    }

    Ok((changed, deleted, updated))
}
