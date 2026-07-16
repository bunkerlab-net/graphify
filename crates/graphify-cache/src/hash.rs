//! File-content hashing with a stat-index fastpath.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::CacheError;
use crate::paths::{normalize_path, posix_string};
use crate::stat_index::{StatEntry, ensure_stat_index, lock_index};

/// True if `line` (excluding its trailing `\n`) is a frontmatter delimiter:
/// exactly three dashes, optional trailing spaces/tabs, and an optional
/// single trailing `\r`. Mirrors the regex `^---[ \t]*\r?$` (#1259).
fn is_delim_line(line: &[u8]) -> bool {
    let Some(rest) = line.strip_prefix(b"---") else {
        return false;
    };
    // Allow a single trailing carriage return (CRLF line endings).
    let rest = rest.strip_suffix(b"\r").unwrap_or(rest);
    rest.iter().all(|&b| b == b' ' || b == b'\t')
}

/// Strip YAML frontmatter from Markdown content, returning only the body.
///
/// A frontmatter delimiter is a *whole* line of exactly three dashes (with
/// optional trailing whitespace). Substring checks like `startswith("---")`
/// also match `----` thematic breaks and `--- text` prose, silently dropping
/// everything above them from the hash (#1259). The opener must be the first
/// line; the body begins right after the closing `---` (byte-identical with
/// the historical slice for well-formed frontmatter so cache hashes do not
/// churn).
///
/// Operates on raw bytes so non-UTF-8 content in the Markdown body is
/// preserved verbatim instead of being passed through
/// `decode(errors="replace")` like Python does. Files without a well-formed
/// frontmatter block are returned unchanged.
#[must_use]
pub fn body_content(content: &[u8]) -> Vec<u8> {
    let first_end = content
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(content.len());
    if !is_delim_line(&content[..first_end]) {
        return content.to_vec();
    }
    // Opener with no following line can have no closer.
    if first_end >= content.len() {
        return content.to_vec();
    }
    let mut pos = first_end + 1;
    while pos < content.len() {
        let rel = content[pos..].iter().position(|&b| b == b'\n');
        let line_end = rel.map_or(content.len(), |i| pos + i);
        if is_delim_line(&content[pos..line_end]) {
            // Slice right after the closing `---` (keeps any trailing
            // whitespace on the closer line, matching the Python slice).
            return content[pos + 3..].to_vec();
        }
        match rel {
            Some(_) => pos = line_end + 1,
            None => break,
        }
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
/// `cache_root` (when `Some`) relocates the stat-index fastpath file to the
/// cache location rather than the key anchor `root`, so an `extract <corpus>
/// --out <elsewhere>` run leaves no `graphify-out/cache/stat-index.json` inside
/// the analysed source tree (#1774 completion). `None` anchors it at `root`.
///
/// # Errors
///
/// Returns [`CacheError::NotAFile`] if `path` is not a regular file, or
/// [`CacheError::Io`] on read failure.
pub fn file_hash<P: AsRef<Path>>(
    path: P,
    root: &Path,
    cache_root: Option<&Path>,
) -> Result<String, CacheError> {
    let p = normalize_path(path.as_ref());
    let root = normalize_path(root);
    if !p.is_file() {
        return Err(CacheError::NotAFile(p));
    }

    let key = ensure_stat_index(&root, cache_root);
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
        let index = lock_index();
        // Word-count-only entries carry no hash — require one for the fastpath.
        if let Some(state) = index.roots.get(&key)
            && let Some(entry) = state.entries.get(&abs_key_str)
            && entry.size == size
            && entry.mtime_ns == mtime_ns
            && let Some(hash) = &entry.hash
        {
            return Ok(hash.clone());
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
        let mut index = lock_index();
        let state = index.roots.entry(key).or_default();
        // Preserve a co-located word_count when the (size, mtime_ns) signature is
        // still current; otherwise replace the stale entry outright.
        match state.entries.get_mut(&abs_key_str) {
            Some(entry) if entry.size == size && entry.mtime_ns == mtime_ns => {
                entry.hash = Some(digest.clone());
            }
            _ => {
                state.entries.insert(
                    abs_key_str,
                    StatEntry {
                        size,
                        mtime_ns,
                        hash: Some(digest.clone()),
                        word_count: None,
                    },
                );
            }
        }
        state.dirty = true;
    }

    Ok(digest)
}

/// Word count with the same `(size, mtime_ns)` stat-fastpath cache as
/// [`file_hash`], persisted in the shared stat index.
///
/// `detect()` counts words in every PDF/docx/text file to size the corpus,
/// re-parsing every binary on each run — minutes on a large docs corpus even
/// when only a handful of files changed (#1656). This caches the count against
/// the file's stat signature so an unchanged file is counted once and read from
/// the index thereafter. `compute` produces the count on a miss. A file that
/// can't be stat'd simply recomputes and isn't cached — correct, just not
/// accelerated.
pub fn cached_word_count<P: AsRef<Path>>(
    path: P,
    root: &Path,
    compute: impl FnOnce(&Path) -> u64,
    cache_root: Option<&Path>,
) -> u64 {
    let p = normalize_path(path.as_ref());
    let root = normalize_path(root);
    let key = ensure_stat_index(&root, cache_root);
    let abs_key = p.canonicalize().unwrap_or_else(|_| p.clone());
    let abs_key_str = abs_key.to_string_lossy().to_string();

    let Ok(meta) = p.metadata() else {
        return compute(&p); // can't stat (e.g. an unreachable long path) — recompute, don't cache
    };
    let size = meta.len();
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or_default();

    {
        let index = lock_index();
        if let Some(state) = index.roots.get(&key)
            && let Some(entry) = state.entries.get(&abs_key_str)
            && entry.size == size
            && entry.mtime_ns == mtime_ns
            && let Some(wc) = entry.word_count
        {
            return wc;
        }
    }

    let wc = compute(&p);

    {
        let mut index = lock_index();
        let state = index.roots.entry(key).or_default();
        // Augment an existing hash entry in place when its signature is current;
        // otherwise create a word-count-only entry (no hash).
        match state.entries.get_mut(&abs_key_str) {
            Some(entry) if entry.size == size && entry.mtime_ns == mtime_ns => {
                entry.word_count = Some(wc);
            }
            _ => {
                state.entries.insert(
                    abs_key_str,
                    StatEntry {
                        size,
                        mtime_ns,
                        hash: None,
                        word_count: Some(wc),
                    },
                );
            }
        }
        state.dirty = true;
    }

    wc
}
