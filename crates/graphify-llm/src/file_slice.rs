//! Intra-file slicing for oversized text documents (#1369).
//!
//! The extraction packer ([`crate::pack_chunks_by_tokens`]) treats each file as
//! atomic and [`crate::read_files`] caps every file at [`crate::FILE_CHAR_CAP`]
//! characters, so a document larger than that cap had everything past the cap
//! silently dropped — the model never saw it, and nothing in the adaptive-retry
//! path could recover it.
//!
//! This module splits an oversized *splittable text* document (Markdown, plain
//! text, reStructuredText) into contiguous [`FileSlice`] units at heading /
//! paragraph / line boundaries so the whole file gets extracted across several
//! units. Every slice of a file reports the **parent file path** as its source,
//! so the resulting nodes are never fragmented per-slice — they merge by
//! `source_file` exactly as if the file had been extracted in one pass.
//!
//! Only plain-text documents are sliced: code files need whole-symbol context,
//! and PDFs/images are read through their own extractors and have no char-offset
//! model. Mirrors `graphify-py/graphify/file_slice.py`.
//!
//! Offsets are byte offsets into the (lossy-UTF-8) file contents and always land
//! on `char` boundaries, so every slice is a valid `&str` range. The crate's
//! ASCII-dominated corpora make byte offsets coincide with Python's character
//! offsets in practice.

use std::path::{Path, PathBuf};

/// Plain-text document types where boundary-based slicing is meaningful and
/// where reading is a straight `read_text` (so a char range matches the bytes
/// the model is shown). Deliberately excludes code (`.py`, `.ts`, ...) and
/// binary docs (`.pdf`) — those are never sliced.
const SPLITTABLE_TEXT_SUFFIXES: &[&str] = &["md", "mdx", "markdown", "txt", "rst"];

/// Boundary preferences, strongest first. A Markdown heading (`\n#`) keeps a
/// section with its title; a blank line keeps a paragraph intact; a bare newline
/// avoids cutting mid-line. If none is found in the window we hard-cut.
const BOUNDARY_SEPARATORS: &[&str] = &["\n#", "\n\n", "\n"];

/// A contiguous `[start, end)` byte range of a splittable text file.
///
/// `index`/`total` are for logging only. `path` is the real file on disk; the
/// slice always reports `path` as its source so slices don't fragment the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSlice {
    /// Parent file on disk that this slice belongs to.
    pub path: PathBuf,
    /// Inclusive start byte offset into the file contents.
    pub start: usize,
    /// Exclusive end byte offset into the file contents.
    pub end: usize,
    /// Zero-based slice index within the parent file (logging only).
    pub index: usize,
    /// Total number of slices the parent file was split into (logging only).
    pub total: usize,
}

/// A unit of extraction work: either a whole file or one slice of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unit {
    /// A whole file extracted in one pass.
    Whole(PathBuf),
    /// One slice of an oversized splittable text file.
    Slice(FileSlice),
}

/// The on-disk path a unit belongs to (the parent file for a slice).
#[must_use]
pub fn unit_path(unit: &Unit) -> &Path {
    match unit {
        Unit::Whole(p) => p,
        Unit::Slice(fs) => &fs.path,
    }
}

/// `true` for plain-text document types that may be sliced.
#[must_use]
pub fn is_splittable_text(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| SPLITTABLE_TEXT_SUFFIXES.contains(&ext.as_str()))
}

/// Largest byte index `<= idx` that is a `char` boundary of `s`.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest byte index `>= idx` that is a `char` boundary of `s`.
fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Return a cut index in `(start, end]` at the strongest nearby boundary.
///
/// Searches the window `text[start..end]` for the latest heading, then blank
/// line, then newline, and returns the index just *after* it (a heading cuts
/// just *before* the `#` so the heading leads the next slice). Falls back to a
/// hard cut at `end` when the window has no usable boundary, which still makes
/// forward progress because `end > start`.
fn best_cut(text: &str, start: usize, end: usize) -> usize {
    let window = &text[start..end];
    for sep in BOUNDARY_SEPARATORS {
        // A boundary strictly inside the window (non-empty previous slice).
        if let Some(idx) = window.rfind(*sep)
            && idx > 0
        {
            if *sep == "\n#" {
                // Keep the newline with the previous slice; heading leads next.
                return start + idx + 1;
            }
            return start + idx + sep.len();
        }
    }
    end
}

/// Contiguous `(start, end)` byte ranges covering all of `text`, each `<= max_chars`.
///
/// Ranges are gap-free and non-overlapping, so concatenating the slices
/// reproduces `text` exactly — no content is dropped.
#[must_use]
pub fn slice_boundaries(text: &str, max_chars: usize) -> Vec<(usize, usize)> {
    let n = text.len();
    if n <= max_chars {
        return vec![(0, n)];
    }
    let mut bounds: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0;
    while pos < n {
        let hard = floor_char_boundary(text, (pos + max_chars).min(n));
        let mut end = if hard < n {
            best_cut(text, pos, hard)
        } else {
            n
        };
        if end <= pos {
            // Defensive: never stall. `hard` already makes progress for any
            // sane `max_chars`; the ceil step covers a lone char wider than it.
            end = hard;
            if end <= pos {
                end = ceil_char_boundary(text, pos + 1).min(n);
            }
        }
        bounds.push((pos, end));
        pos = end;
    }
    bounds
}

/// Replace each oversized splittable-text file with a list of [`FileSlice`]s.
///
/// Files at or below `max_chars` (and all non-splittable files) pass through
/// unchanged as [`Unit::Whole`], so behaviour is identical for everything that
/// already fit. Unreadable files pass through untouched (the reader handles the
/// error).
#[must_use]
pub fn expand_oversized_files(files: &[PathBuf], max_chars: usize) -> Vec<Unit> {
    let mut out: Vec<Unit> = Vec::with_capacity(files.len());
    for f in files {
        if !is_splittable_text(f) {
            out.push(Unit::Whole(f.clone()));
            continue;
        }
        let Some(text) = read_file_lossy(f) else {
            out.push(Unit::Whole(f.clone()));
            continue;
        };
        if text.len() <= max_chars {
            out.push(Unit::Whole(f.clone()));
            continue;
        }
        let ranges = slice_boundaries(&text, max_chars);
        let total = ranges.len();
        for (index, (start, end)) in ranges.into_iter().enumerate() {
            out.push(Unit::Slice(FileSlice {
                path: f.clone(),
                start,
                end,
                index,
                total,
            }));
        }
    }
    out
}

/// Read just this slice's bytes from its parent file (lossy UTF-8).
///
/// Returns `None` when the parent file cannot be read, mirroring the `OSError`
/// path in Python's `read_slice_text` that callers translate into a skip.
#[must_use]
pub fn read_slice_text(fs: &FileSlice) -> Option<String> {
    let text = read_file_lossy(&fs.path)?;
    Some(text.get(fs.start..fs.end).unwrap_or("").to_string())
}

/// Split a slice into two halves at a newline near its midpoint, or `None`.
///
/// Used by the adaptive-retry path when a single slice still overflows the
/// model's output: halving it produces a smaller response. Returns `None` when
/// the slice is already too small to split meaningfully.
#[must_use]
pub fn bisect_slice(fs: &FileSlice) -> Option<(FileSlice, FileSlice)> {
    if fs.end - fs.start <= 1 {
        return None;
    }
    let text = read_file_lossy(&fs.path)?;
    let mid = floor_char_boundary(&text, usize::midpoint(fs.start, fs.end));
    // First newline in `[mid, end)`, if any.
    let nl = text
        .get(mid..fs.end)
        .and_then(|w| w.find('\n'))
        .map(|i| mid + i);
    let cut = match nl {
        Some(nl_idx) if fs.start < nl_idx + 1 && nl_idx + 1 < fs.end => nl_idx + 1,
        _ => mid,
    };
    if !(fs.start < cut && cut < fs.end) {
        return None;
    }
    let left = FileSlice {
        path: fs.path.clone(),
        start: fs.start,
        end: cut,
        index: fs.index,
        total: fs.total,
    };
    let right = FileSlice {
        path: fs.path.clone(),
        start: cut,
        end: fs.end,
        index: fs.index,
        total: fs.total,
    };
    Some((left, right))
}

/// Read a file as lossy UTF-8, mirroring `read_text(errors="replace")`.
fn read_file_lossy(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}
