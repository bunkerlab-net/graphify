//! Entity deduplication pipeline for graphify knowledge graphs.
//!
//! Pipeline:
//! 1. Exact normalization (same label in same file → merge)
//! 2. MinHash/LSH blocking → Jaro-Winkler verification → community
//!    boost
//! 3. Optional LLM tiebreaker for the 75–92 score zone
//! 4. Union-Find merge + edge rewire
//!
//! Ports `graphify-py/graphify/dedup.py`.

mod api;
mod backend;
mod error;
pub mod merge;
pub mod minhash;
pub mod score;

pub use api::deduplicate_entities;
pub use backend::{DedupLlmBackend, JudgeResult, NoOpBackend};
pub use error::DedupError;
pub use score::{entropy, is_variant_pair, norm, shingles, short_label_blocked};
