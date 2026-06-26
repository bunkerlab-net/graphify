//! Path conventions for the cache.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};

use crate::error::CacheError;

/// Version that namespaces the AST cache. AST entries are the output of
/// graphify's own extractor code, so they are only valid for the version
/// that wrote them; bumping the package invalidates them. The semantic cache
/// is deliberately *not* versioned (re-extraction costs LLM calls).
pub const EXTRACTOR_VERSION: &str = env!("CARGO_PKG_VERSION");

/// AST version directories already swept this process — cleanup runs once per
/// `(base, version)` to avoid re-listing the directory on every cached file.
static CLEANED_AST_DIRS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Resolve the absolute path to the graphify output directory relative to
/// `root`.
///
/// If the configured output path is absolute, it is returned as-is;
/// otherwise it is joined to the canonicalised `root`. When
/// `root.canonicalize()` fails (e.g. the directory does not yet exist
/// during initial setup) the un-canonicalised `root` is used; the
/// downstream `fs::create_dir_all` call in [`cache_dir`] will surface
/// the underlying I/O error if the path is unusable.
pub(crate) fn out_base(root: &Path) -> PathBuf {
    let out = graphify_security::graphify_out();
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

/// Return the cache directory for `kind`, creating it if it does not exist.
///
/// AST entries live under a per-version subdirectory
/// (`graphify-out/cache/ast/v{version}/`) because they depend on extractor
/// code, not just file contents; semantic entries live unversioned in
/// `graphify-out/cache/semantic/`.
///
/// # Errors
///
/// Returns [`CacheError::Io`] if the directory could not be created.
pub fn cache_dir(root: &Path, kind: &str) -> Result<PathBuf, CacheError> {
    cache_dir_versioned(root, kind, EXTRACTOR_VERSION)
}

/// Like [`cache_dir`] but with the AST namespace version supplied explicitly.
/// Used both by [`cache_dir`] (passing [`EXTRACTOR_VERSION`]) and by tests
/// that need to simulate an upgrade.
///
/// # Errors
///
/// Returns [`CacheError::Io`] if the directory could not be created.
pub fn cache_dir_versioned(root: &Path, kind: &str, version: &str) -> Result<PathBuf, CacheError> {
    let mut d = out_base(root).join("cache").join(kind);
    if kind == "ast" {
        d = d.join(format!("v{version}"));
        if let Some(parent) = d.parent() {
            cleanup_stale_ast_entries(parent, &d);
        }
    }
    fs::create_dir_all(&d)?;
    Ok(d)
}

/// Remove AST cache entries left behind by other graphify versions.
///
/// Sweeps sibling `v*/` directories and unversioned `*.json` entries (the
/// pre-versioning layout) under `cache/ast/`. Runs at most once per version
/// directory per process. Best-effort: filesystem failures are ignored and
/// stragglers are retried on the next run.
fn cleanup_stale_ast_entries(ast_base: &Path, current_dir: &Path) {
    let key = current_dir.to_string_lossy().into_owned();
    {
        let mut guard = CLEANED_AST_DIRS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !guard.insert(key) {
            return;
        }
    }
    let Ok(entries) = fs::read_dir(ast_base) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child == *current_dir {
            continue;
        }
        if child.is_dir() {
            if child
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('v'))
            {
                let _ = fs::remove_dir_all(&child);
            }
        } else if child.extension().and_then(|e| e.to_str()) == Some("json") {
            let _ = fs::remove_file(&child);
        }
    }
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
                if !out.ends_with('/') {
                    out.push('/');
                }
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
