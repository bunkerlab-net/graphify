//! Graph loading and the top-level [`run_benchmark`] driver.

use std::path::Path;

use serde_json::Value;

use graphify_build::{Graph, build_from_json};

use crate::error::BenchmarkError;
use crate::subgraph::query_subgraph_tokens;
use crate::types::{BenchmarkResult, QuestionResult, SAMPLE_QUESTIONS};

/// Load a graph from a `node_link` JSON file (as written by `NetworkX`).
///
/// # Errors
///
/// Returns [`BenchmarkError`] on I/O failure, JSON parse failure, or if
/// the graph data cannot be assembled by `graphify-build`.
pub fn load_graph(path: &Path) -> Result<Graph, BenchmarkError> {
    let text = std::fs::read_to_string(path)?;
    let mut data: Value = serde_json::from_str(&text)?;

    // `node_link_data` writes nodes under `"nodes"` and edges under `"links"`.
    // `build_from_json` already canonicalises `links` → `edges`, so we pass it
    // straight through. We reconstruct a minimal extraction dict if needed.
    if let Some(obj) = data.as_object_mut()
        && !obj.contains_key("edges")
        && let Some(links) = obj.remove("links")
    {
        obj.insert("edges".to_string(), links);
    }

    let graph = build_from_json(data, false, None)?;
    Ok(graph)
}

/// Run the token-reduction benchmark against a graph JSON file.
///
/// `corpus_words` is the total word count of the corpus. When `None` a
/// rough estimate of `node_count × 50` is used (matches the Python
/// fallback). `questions` defaults to [`SAMPLE_QUESTIONS`] when `None`.
///
/// Returns `None` when no sample questions matched any node in the
/// graph (the Python equivalent returns `{"error": "..."}` in that
/// case).
///
/// # Errors
///
/// Propagates [`BenchmarkError`] from loading the graph file.
pub fn run_benchmark(
    graph_path: &Path,
    corpus_words: Option<usize>,
    questions: Option<&[&str]>,
) -> Result<Option<BenchmarkResult>, BenchmarkError> {
    let graph = load_graph(graph_path)?;

    let corpus_words = corpus_words.unwrap_or_else(|| graph.node_count() * 50);
    // words → tokens: 100 words ≈ 133 tokens (same integer arithmetic as Python).
    let corpus_tokens = corpus_words * 100 / 75;

    let qs = questions.unwrap_or(SAMPLE_QUESTIONS);
    let mut per_question: Vec<QuestionResult> = Vec::new();
    for &q in qs {
        let qt = query_subgraph_tokens(&graph, q, 3);
        if qt > 0 {
            #[allow(clippy::cast_precision_loss)]
            // reason: corpus_tokens and qt are usize; precision loss at extreme sizes
            // is acceptable for a display-only approximation.
            let reduction = (corpus_tokens as f64 / qt as f64 * 10.0).round() / 10.0;
            per_question.push(QuestionResult {
                question: q.to_string(),
                query_tokens: qt,
                reduction,
            });
        }
    }

    if per_question.is_empty() {
        return Ok(None);
    }

    let avg_query_tokens =
        per_question.iter().map(|p| p.query_tokens).sum::<usize>() / per_question.len();

    #[allow(clippy::cast_precision_loss)]
    // reason: same as above — display approximation only.
    let reduction_ratio = if avg_query_tokens > 0 {
        (corpus_tokens as f64 / avg_query_tokens as f64 * 10.0).round() / 10.0
    } else {
        0.0
    };

    Ok(Some(BenchmarkResult {
        corpus_tokens,
        corpus_words,
        nodes: graph.node_count(),
        edges: graph.edge_count(),
        avg_query_tokens,
        reduction_ratio,
        per_question,
    }))
}
