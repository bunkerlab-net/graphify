//! Corpus-vs-subgraph token benchmark.
//!
//! Ports `graphify-py/graphify/benchmark.py`. Measures how much context
//! graphify saves vs a naïve full-corpus approach by comparing corpus tokens
//! against the tokens in a BFS subgraph extracted for sample questions.

use std::fmt::Write as _;
use std::path::Path;

use graphify_build::{Graph, build_from_json};
use serde_json::Value;
use thiserror::Error;

/// Approximate chars-per-token ratio (standard approximation).
const CHARS_PER_TOKEN: usize = 4;

/// Default sample questions used when none are provided.
pub const SAMPLE_QUESTIONS: &[&str] = &[
    "how does authentication work",
    "what is the main entry point",
    "how are errors handled",
    "what connects the data layer to the api",
    "what are the core abstractions",
];

/// Errors that can occur during benchmarking.
#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("I/O error reading graph file: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Build(#[from] graphify_build::BuildError),
}

/// Per-question benchmark result.
#[derive(Debug, Clone)]
pub struct QuestionResult {
    pub question: String,
    pub query_tokens: usize,
    /// `corpus_tokens / query_tokens`, rounded to one decimal place.
    pub reduction: f64,
}

/// Successful benchmark result.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub corpus_tokens: usize,
    pub corpus_words: usize,
    pub nodes: usize,
    pub edges: usize,
    pub avg_query_tokens: usize,
    /// `corpus_tokens / avg_query_tokens`, rounded to one decimal place.
    pub reduction_ratio: f64,
    pub per_question: Vec<QuestionResult>,
}

/// Estimate the number of tokens in a text string.
///
/// Uses the standard approximation of 4 chars per token, with a minimum of 1.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() / CHARS_PER_TOKEN).max(1)
}

/// Run BFS from the best-matching nodes and return the estimated token count
/// for the resulting subgraph context.
///
/// Matches terms (words longer than 2 chars) against node labels. The top-3
/// scoring nodes seed the BFS; `depth` controls how many hops to expand.
/// Returns 0 when no nodes match the query terms.
#[must_use]
pub fn query_subgraph_tokens(graph: &Graph, question: &str, depth: usize) -> usize {
    let terms: Vec<String> = question
        .split_whitespace()
        .filter(|t| t.len() > 2)
        .map(str::to_lowercase)
        .collect();

    // Score each node by how many query terms appear in its label.
    let mut scored: Vec<(usize, &str)> = graph
        .nodes()
        .filter_map(|(nid, data)| {
            let label = data
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            let score = terms.iter().filter(|t| label.contains(t.as_str())).count();
            if score > 0 {
                Some((score, nid.as_str()))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));

    let start_nodes: Vec<&str> = scored.iter().take(3).map(|(_, nid)| *nid).collect();
    if start_nodes.is_empty() {
        return 0;
    }

    // BFS expansion.
    let mut visited: indexmap::IndexSet<&str> = start_nodes.iter().copied().collect();
    let mut frontier: indexmap::IndexSet<&str> = start_nodes.iter().copied().collect();
    let mut edges_seen: Vec<(&str, &str)> = Vec::new();

    for _ in 0..depth {
        let mut next_frontier: indexmap::IndexSet<&str> = indexmap::IndexSet::new();
        for &n in &frontier {
            // Iterate over all edges to find neighbors (undirected semantics).
            for edge in graph.edges() {
                let neighbor = if edge.source == n {
                    Some(edge.target.as_str())
                } else if edge.target == n {
                    Some(edge.source.as_str())
                } else {
                    None
                };
                if let Some(nb) = neighbor
                    && !visited.contains(nb)
                {
                    next_frontier.insert(nb);
                    edges_seen.push((n, nb));
                }
            }
        }
        visited.extend(next_frontier.iter().copied());
        frontier = next_frontier;
    }

    // Build the context text exactly as Python does.
    let mut lines: Vec<String> = Vec::new();
    for nid in &visited {
        if let Some(data) = graph.node_data(nid) {
            let label = data.get("label").and_then(Value::as_str).unwrap_or(nid);
            let src = data
                .get("source_file")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let loc = data
                .get("source_location")
                .and_then(Value::as_str)
                .unwrap_or_default();
            lines.push(format!("NODE {label} src={src} loc={loc}"));
        }
    }
    for &(u, v) in &edges_seen {
        if visited.contains(u) && visited.contains(v) {
            let u_label = graph
                .node_data(u)
                .and_then(|d| d.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(u);
            let v_label = graph
                .node_data(v)
                .and_then(|d| d.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(v);
            let relation = graph
                .edge_data(u, v)
                .and_then(|d| d.get("relation"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            lines.push(format!("EDGE {u_label} --{relation}--> {v_label}"));
        }
    }

    estimate_tokens(&lines.join("\n"))
}

/// Load a graph from a `node_link` JSON file (as written by `networkx`).
///
/// # Errors
///
/// Returns [`BenchmarkError`] on I/O failure, JSON parse failure, or if the
/// graph data cannot be assembled by `graphify-build`.
pub fn load_graph(path: &Path) -> Result<Graph, BenchmarkError> {
    let text = std::fs::read_to_string(path)?;
    let mut data: Value = serde_json::from_str(&text)?;

    // `node_link_data` writes nodes under `"nodes"` and edges under `"links"`.
    // `build_from_json` already canonicalises `links` → `edges`, so we pass it
    // straight through.  We reconstruct a minimal extraction dict if needed.
    if let Some(obj) = data.as_object_mut() {
        // Promote top-level `links` to `edges` so build_from_json handles it.
        if !obj.contains_key("edges")
            && let Some(links) = obj.remove("links")
        {
            obj.insert("edges".to_string(), links);
        }
    }

    let graph = build_from_json(data, false, None)?;
    Ok(graph)
}

/// Run the token-reduction benchmark against a graph JSON file.
///
/// `corpus_words` is the total word count of the corpus. When `None` a rough
/// estimate of `node_count × 50` is used (matches Python fallback).
/// `questions` defaults to [`SAMPLE_QUESTIONS`] when `None`.
///
/// Returns `None` when no sample questions matched any node in the graph (the
/// Python equivalent returns `{"error": "..."}` in that case).
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

/// Return a horizontal rule of `width` box-drawing characters.
///
/// Mirrors Python `_hr()`. Always uses `─` (U+2500) — Rust's stdout is always
/// UTF-8 so no encoding fallback is needed.
#[must_use]
pub fn hr(width: usize) -> String {
    "─".repeat(width)
}

/// Format a benchmark result as a human-readable string.
///
/// Mirrors Python `print_benchmark`. Returns the error message when `result`
/// is `None` (i.e. no nodes matched).
#[must_use]
pub fn format_benchmark(result: Option<&BenchmarkResult>) -> String {
    let Some(r) = result else {
        return "Benchmark error: No matching nodes found for sample questions. Build the graph first.\n".to_string();
    };

    let mut out = String::new();
    out.push_str("\ngraphify token reduction benchmark\n");
    out.push_str(&hr(50));
    out.push('\n');
    // `write!` on `String` is infallible; the `let _ =` silences the
    // `unused_must_use` warning without hiding real errors.
    let _ = writeln!(
        out,
        "  Corpus:          {} words → ~{} tokens (naive)",
        format_with_commas(r.corpus_words),
        format_with_commas(r.corpus_tokens),
    );
    let _ = writeln!(
        out,
        "  Graph:           {} nodes, {} edges",
        format_with_commas(r.nodes),
        format_with_commas(r.edges),
    );
    let _ = writeln!(
        out,
        "  Avg query cost:  ~{} tokens",
        format_with_commas(r.avg_query_tokens),
    );
    let _ = writeln!(
        out,
        "  Reduction:       {}x fewer tokens per query",
        r.reduction_ratio,
    );
    out.push_str("\n  Per question:\n");
    for p in &r.per_question {
        let truncated = if p.question.len() > 55 {
            &p.question[..55]
        } else {
            &p.question
        };
        let _ = writeln!(out, "    [{}x] {truncated}", p.reduction);
    }
    out.push('\n');
    out
}

/// Format a number with comma thousands separators (e.g. `1_234_567` → `"1,234,567"`).
#[must_use]
fn format_with_commas(n: usize) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let offset = bytes.len() % 3;
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (i % 3 == offset) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}
