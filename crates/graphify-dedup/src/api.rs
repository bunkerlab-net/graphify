//! Public entry point: [`deduplicate_entities`].

use indexmap::IndexMap;
use serde_json::Value;

use crate::backend::DedupLlmBackend;
use crate::error::DedupError;
use crate::merge;

/// Deduplicate near-identical entities in a knowledge graph.
///
/// # Arguments
///
/// * `nodes` — list of node objects, each with at minimum
///   `{"id": str, "label": str}`.
/// * `edges` — list of edge objects with either
///   `{"source": str, "target": str, ...}` or
///   `{"from": str, "to": str, ...}` endpoint keys (both are accepted).
/// * `communities` — mapping of `node_id → community_id` (from the
///   cluster step).
/// * `dedup_llm_backend` — optional LLM backend for ambiguous-pair
///   resolution. Pass `None` (or a [`crate::NoOpBackend`]) to skip LLM
///   disambiguation.
///
/// # Errors
///
/// Returns [`DedupError::MultipleRepos`] when nodes span more than one
/// `repo` field value. Returns [`DedupError::EmptyGroup`] if an
/// internal group ends up empty (should not happen with well-formed
/// input).
pub fn deduplicate_entities(
    nodes: &[Value],
    edges: &[Value],
    communities: &IndexMap<String, i64>,
    dedup_llm_backend: Option<&dyn DedupLlmBackend>,
) -> Result<(Vec<Value>, Vec<Value>), DedupError> {
    merge::run(nodes, edges, communities, dedup_llm_backend)
}
