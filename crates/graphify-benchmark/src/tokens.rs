//! Token-count estimation primitives.

/// Approximate chars-per-token ratio (standard approximation used by
/// every major tokenizer at very rough granularity).
const CHARS_PER_TOKEN: usize = 4;

/// Estimate the number of tokens in a text string.
///
/// Uses the standard approximation of 4 chars per token, with a minimum
/// of 1 so an empty string still counts as one token (matches the
/// Python reference).
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() / CHARS_PER_TOKEN).max(1)
}
