//! File reading for extraction prompts.
//!
//! Extracted from `lib.rs` to isolate `read_files`, which formats file
//! contents into the `=== rel ===\n{content}` prompt sections consumed by
//! all extraction backends.

use std::path::{Path, PathBuf};

use crate::FILE_CHAR_CAP;

/// Read and format file contents for the extraction prompt.
///
/// Each file is capped at [`FILE_CHAR_CAP`] chars and wrapped in
/// `=== {rel} ===\n{content}` sections separated by blank lines.
#[must_use]
pub fn read_files(paths: &[PathBuf], root: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in paths {
        let rel = p.strip_prefix(root).unwrap_or(p.as_path());
        let Ok(content) = std::fs::read_to_string(p) else {
            continue;
        };
        let capped: String = content.chars().take(FILE_CHAR_CAP).collect();
        parts.push(format!("=== {} ===\n{capped}", rel.display()));
    }
    parts.join("\n\n")
}
