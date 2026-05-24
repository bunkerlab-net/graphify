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
        // When `p` is outside `root` (e.g. an absolute path), only emit the
        // file name so we don't ship absolute filesystem paths to a remote
        // LLM backend.
        let rel: PathBuf = p.strip_prefix(root).map_or_else(
            |_| p.file_name().map_or_else(|| p.clone(), PathBuf::from),
            Path::to_path_buf,
        );
        let content = match std::fs::read_to_string(p) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[graphify] failed to read {} for extraction: {e}",
                    p.display()
                );
                continue;
            }
        };
        let capped: String = content.chars().take(FILE_CHAR_CAP).collect();
        parts.push(format!("=== {} ===\n{capped}", rel.display()));
    }
    parts.join("\n\n")
}
