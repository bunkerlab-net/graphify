//! Corpus-vs-subgraph token benchmark.
//!
//! Ports `graphify-py/graphify/benchmark.py`. Measures how much context
//! graphify saves vs a naïve full-corpus approach by comparing corpus
//! tokens against the tokens in a BFS subgraph extracted for sample
//! questions.

mod error;
mod format;
mod run;
mod subgraph;
mod tokens;
mod types;

pub use error::BenchmarkError;
pub use format::{format_benchmark, hr, print_benchmark};
pub use run::{load_graph, run_benchmark};
pub use subgraph::query_subgraph_tokens;
pub use tokens::estimate_tokens;
pub use types::{BenchmarkResult, QuestionResult, SAMPLE_QUESTIONS};
