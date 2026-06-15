//! Global-graph manifest: the source of truth for which repos are in the
//! global graph and when they were last added.

use std::path::{Path, PathBuf};

use chrono::Utc;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::GlobalError;

/// Per-repo entry recorded in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    /// RFC 3339 UTC timestamp at which the repo was last added.
    pub added_at: String,
    /// Absolute path to the source graph file when the repo was added.
    pub source_path: String,
    /// Node count of the repo's prefixed contribution to the global graph.
    pub node_count: usize,
    /// Edge count of the repo's prefixed contribution to the global graph.
    pub edge_count: usize,
    /// First 16 hex chars of the SHA-256 of the source graph file at the
    /// time of the last `global_add`, used to short-circuit unchanged
    /// repos.
    pub source_hash: String,
}

/// Top-level manifest structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version (currently `1`).
    pub version: u32,
    /// Per-repo entries keyed by repo tag.
    pub repos: IndexMap<String, RepoEntry>,
}

impl Default for Manifest {
    /// Return a version-1 manifest with an empty repo map.
    fn default() -> Self {
        Self {
            version: 1,
            repos: IndexMap::new(),
        }
    }
}

/// Read the manifest from `path`, returning a default empty manifest on
/// any read or parse failure.
///
/// On a parse (or read) failure of an existing file the corrupt file is
/// renamed to `<path>.corrupt.<unix_ts>` and a warning is printed to stderr
/// before the default is returned. This avoids silently wiping every tracked
/// repo on the next write, which a bare "swallow the error and start fresh"
/// would do — the manifest is rewritten in full on every add/remove.
pub(crate) fn load_manifest(path: &Path) -> Manifest {
    if !path.exists() {
        return Manifest::default();
    }
    match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str::<Manifest>(&text).map_err(|e| e.to_string()))
    {
        Ok(manifest) => return manifest,
        Err(exc) => {
            // Back the bad file up and surface the error so the user can
            // recover or report it, rather than losing every tracked repo.
            let ts = Utc::now().timestamp();
            let mut backup_os = path.as_os_str().to_owned();
            backup_os.push(format!(".corrupt.{ts}"));
            let backup = PathBuf::from(backup_os);
            match std::fs::rename(path, &backup) {
                Ok(()) => eprintln!(
                    "[graphify global] manifest at {} failed to parse ({exc}); moved to {} \
                     and starting fresh. Restore from the backup if this was unexpected.",
                    path.display(),
                    backup.display()
                ),
                Err(rename_exc) => eprintln!(
                    "[graphify global] manifest at {} failed to parse ({exc}) and could not be \
                     backed up ({rename_exc}). Starting fresh.",
                    path.display()
                ),
            }
        }
    }
    Manifest::default()
}

/// Serialise `manifest` to pretty-printed JSON and write it to `path`,
/// creating parent directories as needed.
///
/// # Errors
///
/// Returns [`GlobalError::Io`] on filesystem failure or
/// [`GlobalError::Json`] on serialisation failure.
pub(crate) fn save_manifest(path: &Path, manifest: &Manifest) -> Result<(), GlobalError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, text)?;
    Ok(())
}
