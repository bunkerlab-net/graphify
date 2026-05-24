//! [`LlmResponse`] and the [`LlmBackend`] trait that every backend
//! implementation must provide.

use crate::error::LlmError;

/// Structured response from any LLM backend.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    /// Extracted nodes (as raw JSON objects).
    pub nodes: Vec<serde_json::Value>,
    /// Extracted edges (as raw JSON objects).
    pub edges: Vec<serde_json::Value>,
    /// Extracted hyperedges (as raw JSON objects).
    pub hyperedges: Vec<serde_json::Value>,
    /// Number of input tokens billed by the provider.
    pub input_tokens: u64,
    /// Number of output tokens billed by the provider.
    pub output_tokens: u64,
    /// Resolved model identifier (provider-specific).
    pub model: String,
    /// `"stop"` or `"length"` (normalised from backend-specific values).
    pub finish_reason: String,
    /// Wall-clock time taken by this chunk, in seconds.
    pub elapsed_seconds: f64,
    /// Chunk indices (0-based) that failed during parallel extraction.
    pub failed_chunk_indices: Vec<usize>,
}

impl LlmResponse {
    /// Convert to a `serde_json::Value` map (matches the Python dict
    /// shape so consumers can pass either Python or Rust output through
    /// the same downstream code).
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "nodes": self.nodes,
            "edges": self.edges,
            "hyperedges": self.hyperedges,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "model": self.model,
            "finish_reason": self.finish_reason,
        })
    }
}

/// Abstraction over LLM backends.
pub trait LlmBackend: Send + Sync {
    /// Backend identifier (`"claude"`, `"kimi"`, etc.).
    fn name(&self) -> &'static str;

    /// Send `messages` to the model and return a structured response.
    ///
    /// # Errors
    ///
    /// Returns backend-specific errors (HTTP, parse, security, etc.).
    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError>;

    /// Estimate token count for a text snippet.
    fn estimate_tokens(&self, text: &str) -> usize;
}
