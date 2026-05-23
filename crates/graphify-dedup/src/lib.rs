//! Entity deduplication pipeline for graphify knowledge graphs.
//!
//! Pipeline:
//! 1. Exact normalisation (same label in same file → merge)
//! 2. MinHash/LSH blocking → Jaro-Winkler verification → community boost
//! 3. Optional LLM tiebreaker for the 75–92 score zone
//! 4. Union-Find merge + edge rewire
//!
//! Ports `graphify-py/graphify/dedup.py`.

pub mod merge;
pub mod minhash;
pub mod score;

use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;

// ── error type ────────────────────────────────────────────────────────────────

/// Errors produced by the deduplication pipeline.
#[derive(Debug, Error)]
pub enum DedupError {
    /// Nodes span more than one repository; cross-project dedup is disabled.
    #[error(
        "deduplicate_entities: nodes span multiple repos {0:?}. \
         Cross-project dedup is disabled — run dedup per-repo before merging."
    )]
    MultipleRepos(String),

    /// `pick_winner` was called with an empty candidate list.
    #[error("Cannot pick winner from empty list")]
    EmptyGroup,
}

// ── LLM backend trait ─────────────────────────────────────────────────────────

/// Result returned by [`DedupLlmBackend::judge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeResult {
    /// The two labels refer to the same real-world concept — merge them.
    Merge,
    /// The two labels are distinct concepts — do not merge.
    Distinct,
    /// The backend cannot determine the relationship — leave the pair as-is.
    Uncertain,
}

/// Abstraction over an LLM that can judge whether two entity labels refer to
/// the same real-world concept.
///
/// Implement this trait to plug in a real model; use [`NoOpBackend`] (the
/// default) to skip LLM-assisted disambiguation entirely.
pub trait DedupLlmBackend {
    /// Ask whether `a` and `b` are the same concept.
    fn judge(&self, a: &str, b: &str) -> JudgeResult;
}

/// No-op backend — rejects every pair (equivalent to running without LLM).
///
/// This is the default when `dedup_llm_backend` is `None`.
pub struct NoOpBackend;

impl DedupLlmBackend for NoOpBackend {
    /// Always returns [`JudgeResult::Distinct`], effectively disabling LLM-assisted merges.
    fn judge(&self, _a: &str, _b: &str) -> JudgeResult {
        JudgeResult::Distinct
    }
}

// ── public API ────────────────────────────────────────────────────────────────

/// Deduplicate near-identical entities in a knowledge graph.
///
/// # Arguments
///
/// * `nodes` — list of node objects, each with at minimum `{"id": str, "label": str}`.
/// * `edges` — list of edge objects with `{"source": str, "target": str, ...}`.
/// * `communities` — mapping of `node_id → community_id` (from the cluster step).
/// * `dedup_llm_backend` — optional LLM backend for ambiguous-pair resolution.
///   Pass `None` (or a [`NoOpBackend`]) to skip LLM disambiguation.
///
/// # Errors
///
/// Returns [`DedupError::MultipleRepos`] when nodes span more than one repo
/// field value.  Returns [`DedupError::EmptyGroup`] if an internal group ends
/// up empty (should not happen with well-formed input).
pub fn deduplicate_entities(
    nodes: &[Value],
    edges: &[Value],
    communities: &IndexMap<String, i64>,
    dedup_llm_backend: Option<&dyn DedupLlmBackend>,
) -> Result<(Vec<Value>, Vec<Value>), DedupError> {
    merge::run(nodes, edges, communities, dedup_llm_backend)
}

// ── re-exports for tests and integration ─────────────────────────────────────

/// Normalise a label: lowercase + collapse non-alphanumeric runs to space.
///
/// Exposed for testing parity with `graphify.dedup._norm`.
#[must_use]
pub fn norm(label: &str) -> String {
    score::norm(label)
}

/// Shannon entropy in bits/char of the normalised label.
///
/// Exposed for testing parity with `graphify.dedup._entropy`.
#[must_use]
pub fn entropy(label: &str) -> f64 {
    score::entropy(label)
}

/// Return k-gram character shingles of `text` (k = 3 by default).
///
/// Exposed for testing parity with `graphify.dedup._shingles`.
#[must_use]
pub fn shingles(text: &str, k: usize) -> Vec<String> {
    score::shingles(text, k)
}

/// Returns `true` if `a` and `b` are sibling model/SKU variants.
///
/// Exposed for testing parity with `graphify.dedup._is_variant_pair`.
#[must_use]
pub fn is_variant_pair(a: &str, b: &str) -> bool {
    score::is_variant_pair(a, b)
}

/// Returns `true` when a fuzzy merge of `a` and `b` should be blocked.
///
/// Exposed for testing parity with `graphify.dedup._short_label_blocked`.
#[must_use]
pub fn short_label_blocked(a: &str, b: &str, jw_score: f64) -> bool {
    score::short_label_blocked(a, b, jw_score)
}
