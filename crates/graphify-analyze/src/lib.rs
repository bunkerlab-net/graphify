//! Graph analysis.
//!
//! Ports `graphify-py/graphify/analyze.py`.
//!
//! Computes graph-level insights:
//! - God nodes (highest-degree real entities)
//! - Surprising connections (cross-file or cross-community edges)
//! - Suggested questions (LLM prompts derived from graph structure)
//! - Graph diff (what changed between two snapshots)

pub(crate) mod centrality;
pub(crate) mod classify;
pub(crate) mod cross_lang;
pub(crate) mod diff;
pub(crate) mod god_nodes;
pub(crate) mod suggest;
pub(crate) mod surprises;

pub use classify::{file_category, is_concept_node, is_json_key_node};
pub use diff::graph_diff;
pub use god_nodes::god_nodes;
pub use suggest::suggest_questions;
pub use surprises::{SurpriseScoreInput, surprise_score, surprising_connections};
