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
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    match TOKENIZER.as_ref() {
        Some(enc) => enc.encode_with_special_tokens(text).len(),
        None => text.len() / CHARS_PER_TOKEN,
    }
}

/// Estimate tokens for a file's content (already capped at `char_cap` chars)
/// plus the per-file overhead of the `=== rel ===\n` separator.
#[must_use]
pub fn estimate_file_tokens(content: &str, per_file_overhead_chars: usize) -> usize {
    match TOKENIZER.as_ref() {
        Some(enc) => {
            enc.encode_with_special_tokens(content).len()
                + (per_file_overhead_chars / CHARS_PER_TOKEN)
        }
        None => (content.len() + per_file_overhead_chars) / CHARS_PER_TOKEN,
    }
}
