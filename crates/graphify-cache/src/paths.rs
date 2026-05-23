//! Path conventions for the cache.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::CacheError;

/// Output directory name; defaults to `"graphify-out"` and respects the
/// `GRAPHIFY_OUT` environment variable override.
pub(crate) fn graphify_out() -> String {
    std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_string())
}

/// Resolve the absolute path to the graphify output directory relative to
/// `root`.
///
/// If the configured output path is absolute, it is returned as-is;
/// otherwise it is joined to the canonicalised `root`.
pub(crate) fn out_base(root: &Path) -> PathBuf {
    let out = PathBuf::from(graphify_out());
    if out.is_absolute() {
        out
    } else {
        let resolved = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        resolved.join(out)
    }
}

/// Path to the `stat-index.json` file under the cache directory for `root`.
pub(crate) fn stat_index_file(root: &Path) -> PathBuf {
    out_base(root).join("cache").join("stat-index.json")
}

/// Return `graphify-out/cache/{kind}/`, creating it if it does not exist.
///
/// # Errors
///
/// Returns [`CacheError::Io`] if the directory could not be created.
pub fn cache_dir(root: &Path, kind: &str) -> Result<PathBuf, CacheError> {
    let d = out_base(root).join("cache").join(kind);
    fs::create_dir_all(&d)?;
    Ok(d)
}

/// Normalise a path for cache-key consistency.
///
/// On Windows: strip the `\\?\` prefix and lowercase the path so the same
/// file always produces the same key regardless of how it was opened.
/// On Unix: a no-op clone.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    if cfg!(windows) {
        let s = path.to_string_lossy();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        PathBuf::from(s.to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// Convert a `Path` to a POSIX-style forward-slash string so cache hashes
/// are stable across operating systems.
pub(crate) fn posix_string(path: &Path) -> String {
    use std::path::Component::{CurDir, Normal, ParentDir, Prefix, RootDir};
    let mut out = String::new();
    let mut first = true;
    for comp in path.components() {
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
