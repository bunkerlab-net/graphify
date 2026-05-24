//! File-content hashing with a stat-index fastpath.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::CacheError;
use crate::paths::{normalize_path, posix_string};
use crate::stat_index::{StatEntry, ensure_stat_index, lock_index};

/// Strip YAML frontmatter from Markdown content, returning only the body.
///
/// Mirrors Python's `_body_content` semantically, but operates on raw bytes
/// so non-UTF-8 content in the Markdown body is preserved verbatim instead
/// of being passed through `decode(errors="replace")` like Python does.
/// Files without frontmatter are returned unchanged.
#[must_use]
pub fn body_content(content: &[u8]) -> Vec<u8> {
    const OPEN: &[u8] = b"---";
    const CLOSE: &[u8] = b"\n---";
    if let Some(after_open) = content.strip_prefix(OPEN)
        && let Some(end) = after_open.windows(CLOSE.len()).position(|w| w == CLOSE)
    {
        return after_open[end + CLOSE.len()..].to_vec();
    }
    content.to_vec()
}

/// SHA-256 of file contents plus the path relative to `root`.
///
/// Uses a `(size, mtime_ns)` fastpath: if the in-memory stat index has an
/// entry whose `(size, mtime_ns)` matches the file's metadata, the cached
/// hash is returned without re-reading the file. On miss, the hash is
/// computed (Markdown files have their frontmatter stripped first) and
/// stored back into the index.
///
/// # Errors
///
/// Returns [`CacheError::NotAFile`] if `path` is not a regular file, or
/// [`CacheError::Io`] on read failure.
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
