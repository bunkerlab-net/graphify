//! LLM backend trait used to break ties in the 75–92 score zone.

/// Result returned by [`DedupLlmBackend::judge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeResult {
    /// The two labels refer to the same real-world concept — merge
    /// them.
    Merge,
    /// The two labels are distinct concepts — do not merge.
    Distinct,
    /// The backend cannot determine the relationship — leave the pair
    /// as-is.
    Uncertain,
}

/// Abstraction over an LLM that can judge whether two entity labels
/// refer to the same real-world concept.
///
/// Implement this trait to plug in a real model; use [`NoOpBackend`]
/// (the default) to skip LLM-assisted disambiguation entirely.
pub trait DedupLlmBackend {
    /// Ask whether `a` and `b` are the same concept.
    fn judge(&self, a: &str, b: &str) -> JudgeResult;
}

/// No-op backend — rejects every pair (equivalent to running without an
/// LLM).
///
/// This is the default when `dedup_llm_backend` is `None`.
pub struct NoOpBackend;

impl DedupLlmBackend for NoOpBackend {
    /// Always return [`JudgeResult::Distinct`], effectively disabling
    /// LLM-assisted merges.
    fn judge(&self, _a: &str, _b: &str) -> JudgeResult {
        JudgeResult::Distinct
    }
}
