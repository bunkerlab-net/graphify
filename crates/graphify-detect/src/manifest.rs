//! Manifest persistence and incremental detection.
//!
//! Ports `save_manifest`, `load_manifest`, `detect_incremental`, and
//! `_md5_file` from `graphify-py/graphify/detect.py`.

use std::io::Read;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::DetectError;

/// Typed result from [`detect_incremental_with_manifest`].
pub type IncrementalResult = (Vec<PathBuf>, Vec<PathBuf>, IndexMap<String, ManifestEntry>);

/// The on-disk path for the manifest, relative to the scan root.
pub const MANIFEST_PATH: &str = "graphify-out/manifest.json";

// ── Per-file manifest entry ───────────────────────────────────────────────────

/// One entry in the manifest JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub mtime: f64,
    pub ast_hash: String,
    pub semantic_hash: String,
}

/// Normalise a legacy manifest entry value to a `ManifestEntry`.
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
    let digest = ctx.compute();
    format!("{digest:x}")
}

// ── mtime helper ─────────────────────────────────────────────────────────────

/// Get file mtime as an `f64` (seconds + fractional nanoseconds).
fn file_mtime(path: &Path) -> Option<f64> {
    use std::os::unix::fs::MetadataExt;
    let meta = path.metadata().ok()?;
    // Use i64 → f64 cast only for the seconds field.
    // Nanoseconds fit in i32, so no precision loss there.
    #[allow(clippy::cast_precision_loss)]
    let secs = meta.mtime() as f64;
    #[allow(clippy::cast_precision_loss)]
    let nsecs = meta.mtime_nsec() as f64 / 1_000_000_000.0;
    Some(secs + nsecs)
}

// ── Public API ────────────────────────────────────────────────────────────────

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
            map.insert(k.clone(), entry);
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
    let existing = load_manifest_from_path(manifest_path)?;

    // Seed from existing; prune entries for deleted files.
    let mut manifest: IndexMap<String, ManifestEntry> = IndexMap::new();
    for (f, entry) in &existing {
        if Path::new(f).try_exists().unwrap_or(false) {
            manifest.insert(f.clone(), entry.clone());
        }
    }

    for file_list in files.values() {
        for f in file_list {
            let p = Path::new(f);
            let Some(mtime) = file_mtime(p) else {
                continue;
            };
            let h = md5_file(p);
            if h.is_empty() {
                continue;
            }

            let prev = existing.get(f).cloned().unwrap_or(ManifestEntry {
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
            manifest.insert(f.clone(), entry);
        }
    }

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
    let manifest = load_manifest_from_path(manifest_path)?;

    let all_current: Vec<String> = full.files.values().flatten().cloned().collect();

    let mut changed: Vec<PathBuf> = Vec::new();

    for f in &all_current {
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
                } else if (current_mtime - entry.mtime).abs() > f64::EPSILON {
                    // mtime bumped — verify with content hash
                    md5_file(p) != *stored_hash
                } else {
                    false
                }
            }
        };

        if file_changed {
            changed.push(PathBuf::from(f));
        }
    }

    // Files in manifest that no longer exist → deleted
    let current_set: std::collections::HashSet<&str> =
        all_current.iter().map(String::as_str).collect();
    let deleted: Vec<PathBuf> = manifest
        .keys()
        .filter(|k| !current_set.contains(k.as_str()))
        .map(PathBuf::from)
        .collect();

    // Build updated manifest from current state.
    let mut updated: IndexMap<String, ManifestEntry> = manifest.clone();
    for f in &changed {
        let p = f.as_path();
        let mtime = file_mtime(p).unwrap_or(0.0);
        let h = md5_file(p);
        let prev = updated
            .get(f.to_string_lossy().as_ref())
            .cloned()
            .unwrap_or(ManifestEntry {
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
        updated.insert(f.to_string_lossy().into_owned(), entry);
    }
    // Remove deleted files from updated manifest
    for d in &deleted {
        updated.swap_remove(d.to_string_lossy().as_ref());
    }

    Ok((changed, deleted, updated))
}
