//! File reading for extraction prompts.
//!
//! Each file is wrapped in an `<untrusted_source path=… sha256=…>` block and
//! known prompt-injection / chat-template sentinels are defanged, so
//! attacker-controlled source text cannot be confused with the trusted system
//! instructions (#1210). Mirrors `_read_files` / `_wrap_untrusted` /
//! `_neutralise_injection_sentinels` in `graphify-py/graphify/llm.py`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::FILE_CHAR_CAP;

/// Known prompt-injection / chat-template sentinels a hostile source file might
/// embed to break out of the `untrusted_source` block or impersonate a
/// system/role turn. The closing delimiter of our own wrapper is included so a
/// file cannot forge an early `</untrusted_source>` and smuggle instructions out.
#[allow(clippy::expect_used)] // literal pattern; build cannot fail
static INJECTION_SENTINELS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?im)</?untrusted_source\b[^>]*>",
        r"|<\|(?:im_start|im_end|system|user|assistant|endoftext)\|>",
        r"|<<SYS>>|<</SYS>>",
        r"|\[/?INST\]",
        r"|^\s*###?\s*(?:system|instruction)s?\s*:?\s*$",
    ))
    .expect("static injection-sentinel regex")
});

/// Defang known chat-template / jailbreak control tokens in untrusted text.
///
/// Inserts a zero-width space (U+200B) after the first character of each match
/// so the literal token is no longer recognised by any model's template parser
/// or a naive delimiter scan, while keeping the text human-readable. Mirrors
/// `_neutralise_injection_sentinels`.
#[must_use]
pub fn neutralise_injection_sentinels(text: &str) -> String {
    INJECTION_SENTINELS
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let m = &caps[0];
            let mut chars = m.char_indices();
            match chars.next() {
                Some((_, first)) => {
                    let rest = &m[first.len_utf8()..];
                    format!("{first}\u{200b}{rest}")
                }
                None => String::new(),
            }
        })
        .into_owned()
}

/// Return a text-like file's content for the extraction prompt.
///
/// Most files are read directly (lossy UTF-8). PDFs are binary, so reading them
/// as text yields garbage; route them through the detect crate's pypdf-backed
/// extractor instead (#1110). A scanned PDF with no text layer extracts to an
/// empty string, which still produces a reference node rather than noise.
/// Returns `None` only when a non-PDF file cannot be read. Mirrors `_file_to_text`.
fn file_to_text(path: &Path) -> Option<String> {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    {
        return Some(graphify_detect::office::extract_pdf_text(path));
    }
    // Lossy read mirrors Python's `read_text(errors="replace")`.
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// Wrap one file's content in a labelled, hash-stamped untrusted-data block. The
/// sha256 lets a reviewer correlate a suspicious node back to the exact bytes
/// that produced it. Mirrors `_wrap_untrusted`.
#[must_use]
pub fn wrap_untrusted(rel: &str, content: &str) -> String {
    let sha = hex::encode(Sha256::digest(content.as_bytes()));
    let safe = neutralise_injection_sentinels(content);
    format!("<untrusted_source path=\"{rel}\" sha256=\"{sha}\">\n{safe}\n</untrusted_source>")
}

/// Read and format file contents for the extraction prompt.
///
/// Each file is capped at [`FILE_CHAR_CAP`] chars, sentinel-defanged, and
/// wrapped in an `<untrusted_source>` block; sections are separated by blank
/// lines. Mirrors `_read_files`.
#[must_use]
pub fn read_files(paths: &[PathBuf], root: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in paths {
        // When `p` is outside `root` (e.g. an absolute path), only emit the
        // file name so we don't ship absolute filesystem paths to a remote
        // LLM backend (deliberate divergence from graphify-py's `str(p)`).
        let rel: PathBuf = p.strip_prefix(root).map_or_else(
            |_| p.file_name().map_or_else(|| p.clone(), PathBuf::from),
            Path::to_path_buf,
        );
        let Some(content) = file_to_text(p) else {
            eprintln!("[graphify] failed to read {} for extraction", p.display());
            continue;
        };
        let capped: String = content.chars().take(FILE_CHAR_CAP).collect();
        parts.push(wrap_untrusted(&rel.to_string_lossy(), &capped));
    }
    parts.join("\n\n")
}
