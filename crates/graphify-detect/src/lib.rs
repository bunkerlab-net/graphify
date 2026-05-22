//! File discovery + filtering for graphify.
//!
//! Ports `graphify-py/graphify/detect.py`. Provides:
//! - Extension-based and content-based file classification
//! - Gitignore-aware directory walking
//! - Sensitive-file and noise-directory filtering
//! - Manifest persistence for incremental detection
//!
//! # Side effects
//!
//! [`save_manifest`] writes to `graphify-out/manifest.json` under the scan
//! root by default. This path is parameterised by the `manifest_path`
//! argument so callers can override it.

pub mod error;
pub mod extensions;
pub mod ignore;
pub mod manifest;
pub mod sensitive;
pub mod walk;

pub use error::DetectError;
pub use extensions::{CODE_EXTENSIONS, FileType, GOOGLE_WORKSPACE_EXTENSIONS, classify_file};
pub use ignore::{
    could_contain_included_path, find_vcs_root, is_ignored, is_included, load_graphifyignore,
    load_graphifyinclude, parse_gitignore_line,
};
pub use manifest::{
    MANIFEST_PATH, ManifestEntry, detect_incremental_with_manifest, load_manifest_from_path,
    md5_file, save_manifest_to_path,
};
pub use sensitive::{SKIP_DIRS, SKIP_FILES, is_noise_dir, is_sensitive};
pub use walk::{DetectResult, auto_follow_symlinks, collect_files, detect};

use indexmap::IndexMap;
use std::path::{Path, PathBuf};

// ── Stable public API (used by graphify-manifest and downstream crates) ───────

/// Manifest type: ordered map of file path string → `ManifestEntry`.
pub type Manifest = IndexMap<String, ManifestEntry>;

/// Persist a manifest to disk.
///
/// Writes to `<root>/graphify-out/manifest.json` by default.
///
/// # Errors
///
/// Returns `DetectError::Io` on write failure.
pub fn save_manifest(
    files: &IndexMap<String, Vec<String>>,
    manifest_path: &Path,
) -> Result<(), DetectError> {
    save_manifest_to_path(files, manifest_path, "both")
}

/// Load a manifest from disk.
///
/// Returns an empty map on any error (missing file, parse failure, etc.).
///
/// # Errors
///
/// This function always returns `Ok` — errors are swallowed and the map is
/// empty, matching Python's `return {}` on any exception.
pub fn load_manifest(root: &Path) -> Result<Manifest, DetectError> {
    let path = root.join(MANIFEST_PATH);
    load_manifest_from_path(&path)
}

/// Run incremental detection given a previously-saved manifest.
///
/// Returns `(changed_files, deleted_files, updated_manifest)`.
///
/// # Errors
///
/// Returns `DetectError` on I/O or parse failure.
pub fn detect_incremental(
    root: &Path,
    prev: &Manifest,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>, Manifest), DetectError> {
    // Use the standard manifest path under root.
    let manifest_path = root.join(MANIFEST_PATH);

    // If caller passes an in-memory manifest (e.g. from a previous load), write
    // it to a temp location so `detect_incremental_with_manifest` can read it.
    // For the common case where the manifest is already on disk, just use the path.
    let result = if manifest_path.exists() {
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

    Ok(result)
}
