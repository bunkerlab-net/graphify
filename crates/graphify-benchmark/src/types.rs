//! Public result types and the default sample-question list.

/// Default sample questions used when none are provided to
/// [`crate::run_benchmark`].
pub const SAMPLE_QUESTIONS: &[&str] = &[
    "how does authentication work",
    "what is the main entry point",
    "how are errors handled",
    "what connects the data layer to the api",
    "what are the core abstractions",
];

/// Per-question benchmark result.
#[derive(Debug, Clone)]
pub struct QuestionResult {
    /// The question that was evaluated.
    pub question: String,
    /// Estimated tokens of the BFS subgraph context for the question.
    pub query_tokens: usize,
    /// `corpus_tokens / query_tokens`, rounded to one decimal place.
    pub reduction: f64,
}

/// Successful benchmark result.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// Approximate corpus token count; see `crate::tokens::estimate_tokens`
    /// for the word-to-token conversion used.
    pub corpus_tokens: usize,
    /// Word count of the corpus (provided or estimated).
    pub corpus_words: usize,
    /// Node count of the loaded graph.
    pub nodes: usize,
    /// Edge count of the loaded graph.
    pub edges: usize,
    /// Average `query_tokens` across the per-question results.
    pub avg_query_tokens: usize,
    /// `corpus_tokens / avg_query_tokens`, rounded to one decimal place.
    pub reduction_ratio: f64,
    /// Per-question detail.
    pub per_question: Vec<QuestionResult>,
}
