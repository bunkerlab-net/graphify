//! Incremental detection: combine the new file scan with a saved
//! manifest to identify changed / unchanged / deleted files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::error::DetectError;
use crate::manifest::{
    MANIFEST_PATH, ManifestEntry, detect_incremental_with_manifest, load_manifest_from_path,
    save_manifest_to_path,
};
use crate::walk;

/// Manifest type: ordered map of file path string → [`ManifestEntry`].
pub type Manifest = IndexMap<String, ManifestEntry>;

/// Structured return value from [`detect_incremental`].
///
/// Mirrors the richer shape Python callers expect after the PR that
/// added per-type bucketing (`changed_files`, `unchanged_files`) and
/// convenience fields (`new_total`, `incremental`).
pub struct IncrementalDetectResult {
    /// Files whose content hash has changed, keyed by file type
    /// (e.g. `"code"`).
    pub changed_files: IndexMap<String, Vec<String>>,
    /// Paths that existed in the previous manifest but are no longer
    /// on disk.
    pub deleted_files: Vec<PathBuf>,
    /// Updated manifest after the incremental scan.
    pub manifest: Manifest,
    /// Files that are present but unchanged, keyed by file type.
    pub unchanged_files: IndexMap<String, Vec<String>>,
    /// Total number of files seen in the current scan (changed +
    /// unchanged).
    pub new_total: u64,
    /// `true` when a manifest existed (i.e. this was a real
    /// incremental run, not a first scan).
    pub incremental: bool,
}

/// Persist a manifest to disk.
///
/// `kind` controls which hash fields are stamped: `"ast"`,
/// `"semantic"`, or `"both"` (default). Forwarded to
/// [`save_manifest_to_path`].
///
/// # Errors
///
/// Returns [`DetectError::Io`] on write failure.
pub fn save_manifest(
    files: &IndexMap<String, Vec<String>>,
    manifest_path: &Path,
    kind: &str,
) -> Result<(), DetectError> {
    save_manifest_to_path(files, manifest_path, kind)
}

/// Load a manifest from disk.
///
/// Returns an empty map on any error (missing file, parse failure,
/// etc.), matching Python's `return {}` on any exception.
///
/// # Errors
///
/// This function always returns `Ok` — errors are swallowed and the
/// map is empty.
pub fn load_manifest(root: &Path) -> Result<Manifest, DetectError> {
    let path = root.join(MANIFEST_PATH);
    load_manifest_from_path(&path)
}

/// Run incremental detection given a previously-saved manifest.
///
/// Returns [`IncrementalDetectResult`] with changed/unchanged file
/// buckets, deleted files, the updated manifest, and convenience flags.
///
/// # Errors
///
/// Returns [`DetectError`] on I/O or parse failure.
pub fn detect_incremental(
    root: &Path,
    prev: &Manifest,
) -> Result<IncrementalDetectResult, DetectError> {
    let manifest_path = root.join(MANIFEST_PATH);
    let had_manifest = manifest_path.exists() || !prev.is_empty();

    let (changed_paths, deleted_files, manifest) = if manifest_path.exists() {
        detect_incremental_with_manifest(root, &manifest_path, None, "semantic", None)?
    } else if prev.is_empty() {
        // No previous run at all — everything is new.
        let full = walk::detect(root, None, None);
        let changed: Vec<PathBuf> = full.files.values().flatten().map(PathBuf::from).collect();
        (changed, Vec::new(), Manifest::new())
    } else {
        // Caller provided an in-memory manifest — write to a tempfile and delegate.
        let tmp = std::env::temp_dir().join(format!(
            "graphify_manifest_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let json = serde_json::to_string_pretty(prev).map_err(DetectError::Json)?;
        std::fs::write(&tmp, json).map_err(DetectError::Io)?;
        let res = detect_incremental_with_manifest(root, &tmp, None, "semantic", None)?;
        let _ = std::fs::remove_file(&tmp);
        res
    };

    let changed_set: HashSet<PathBuf> = changed_paths.iter().cloned().collect();
    let full = walk::detect(root, None, None);
    let mut changed_files: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut unchanged_files: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut new_total: u64 = 0;
    for (kind, paths) in &full.files {
        for p in paths {
            new_total += 1;
            if changed_set.contains(&PathBuf::from(p)) {
                changed_files
                    .entry(kind.clone())
                    .or_default()
                    .push(p.clone());
            } else {
                unchanged_files
                    .entry(kind.clone())
                    .or_default()
                    .push(p.clone());
            }
        }
    }

    Ok(IncrementalDetectResult {
        changed_files,
        deleted_files,
        manifest,
        unchanged_files,
        new_total,
        incremental: had_manifest,
    })
}
