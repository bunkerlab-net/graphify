//! Token estimation using `tiktoken-rs` (`cl100k_base` encoding).
//!
//! Mirrors the Python `_get_tokenizer()` / `_TOKENIZER` pattern in `llm.py`.
//! Falls back to the `len / 4` heuristic when the encoder is unavailable.

use tiktoken_rs::cl100k_base;

/// Cached encoder — `None` should never occur at compile-time for `cl100k_base`
/// (the data is compiled in), but we match the Python API contract that callers
/// handle `None` gracefully.
static TOKENIZER: std::sync::LazyLock<Option<tiktoken_rs::CoreBPE>> =
    std::sync::LazyLock::new(|| {
        // `cl100k_base()` returns Result; treat any error as "unavailable".
        cl100k_base().ok()
    });

/// Coarse chars-per-token fallback (standard BPE heuristic for English/code).
pub const CHARS_PER_TOKEN: usize = 4;

/// Estimate the number of tokens in `text`.
///
/// Uses `tiktoken-rs` (`cl100k_base`) when available; falls back to
/// `text.len() / CHARS_PER_TOKEN`.
///
/// Uses `encode_ordinary`, which treats tiktoken special-token strings (e.g.
/// `<|endoftext|>`) as ordinary text rather than recognizing them as special
/// ids. This mirrors graphify-py's `encode(..., disallowed_special=())` (#1685):
/// a doc that merely mentions such a string is counted, not special-cased, and
/// never crashes the estimate.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    match TOKENIZER.as_ref() {
        Some(enc) => enc.encode_ordinary(text).len(),
        None => text.len() / CHARS_PER_TOKEN,
    }
}

/// Estimate tokens for a file's content (already capped at `char_cap` chars)
/// plus the per-file overhead of the `=== rel ===\n` separator.
///
/// Like [`estimate_tokens`], uses `encode_ordinary` so special-token strings in
/// the content are tolerated as ordinary text (#1685).
#[must_use]
pub fn estimate_file_tokens(content: &str, per_file_overhead_chars: usize) -> usize {
    match TOKENIZER.as_ref() {
        Some(enc) => {
            enc.encode_ordinary(content).len() + (per_file_overhead_chars / CHARS_PER_TOKEN)
        }
        None => (content.len() + per_file_overhead_chars) / CHARS_PER_TOKEN,
    }
}
