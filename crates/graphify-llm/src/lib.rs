//! LLM router — ports `graphify-py/graphify/llm.py`.
//!
//! Provides:
//! - The [`LlmBackend`] trait with per-backend impls.
//! - A [`router`] factory to get a backend by name.
//! - Token estimation via [`tokenizer`].
//! - File packing, adaptive retry, and corpus-parallel extraction.

pub mod backends;
pub mod bedrock;
pub mod call;
pub mod claude;
pub mod claude_cli;
pub mod deepseek;
pub mod extract;
pub mod gemini;
pub mod kimi;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod parallel;
pub mod parse;
pub mod read;
pub mod retry;
pub mod tokenizer;
pub mod tokens;

// Re-export public symbols to maintain the `graphify_llm::<sym>` path.
pub use backends::{
    BACKENDS, BackendConfig, Pricing, backend_config, detect_backend, format_backend_env_keys,
    get_backend_api_key, router,
};
pub use call::call_llm;
pub use extract::extract_files_direct;
pub use parallel::{ChunkDoneCb, CorpusConfig, extract_corpus_parallel, merge_into};
pub use parse::{empty_fragment, parse_llm_json, response_is_hollow};
pub use read::read_files;
pub use retry::{
    extract_with_adaptive_retry, looks_like_context_exceeded, looks_like_context_exceeded_dyn,
};
pub use tokens::{estimate_cost, estimate_file_tokens, pack_chunks_by_tokens};

use thiserror::Error;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Max chars read from a single file before joining.
pub const FILE_CHAR_CAP: usize = 20_000;
/// Per-file overhead for the `=== rel ===\n` separator.
pub const PER_FILE_OVERHEAD_CHARS: usize = 80;
/// Hard cap on LLM JSON response size before parsing (10 MB).
pub const LLM_JSON_MAX_BYTES: usize = 10 * 1024 * 1024;

/// Extraction system prompt (byte-identical to Python for reproducibility).
pub const EXTRACTION_SYSTEM: &str = "\
You are a graphify semantic extraction agent. Extract a knowledge graph fragment from the files provided.\n\
Output ONLY valid JSON — no explanation, no markdown fences, no preamble.\n\
\n\
Rules:\n\
- EXTRACTED: relationship explicit in source (import, call, citation, reference)\n\
- INFERRED: reasonable inference (shared data structure, implied dependency)\n\
- AMBIGUOUS: uncertain — flag for review, do not omit\n\
\n\
Node ID format: lowercase, only [a-z0-9_], no dots or slashes.\n\
Format: {stem}_{entity} where stem = filename without extension, entity = symbol name (both normalised).\n\
\n\
Output exactly this schema:\n\
{\"nodes\":[{\"id\":\"stem_entity\",\"label\":\"Human Readable Name\",\"file_type\":\"code|document|paper|image|rationale|concept\",\"source_file\":\"relative/path\",\"source_location\":null,\"source_url\":null,\"captured_at\":null,\"author\":null,\"contributor\":null}],\"edges\":[{\"source\":\"node_id\",\"target\":\"node_id\",\"relation\":\"calls|implements|references|cites|conceptually_related_to|shares_data_with|semantically_similar_to\",\"confidence\":\"EXTRACTED|INFERRED|AMBIGUOUS\",\"confidence_score\":1.0,\"source_file\":\"relative/path\",\"source_location\":null,\"weight\":1.0}],\"hyperedges\":[],\"input_tokens\":0,\"output_tokens\":0}\n\
";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors returned by LLM backend calls.
#[derive(Debug, Error)]
pub enum LlmError {
    /// HTTP transport or network failure.
    #[error("HTTP error: {0}")]
    Http(String),

    /// JSON (de)serialisation failure.
    #[error("Parse error: {0}")]
    Parse(String),

    /// The backend returned an empty / filtered response.
    #[error("Empty response: {0}")]
    EmptyResponse(String),

    /// No API key configured.
    #[error("No API key: {0}")]
    NoApiKey(String),

    /// SSRF / URL validation rejected the endpoint.
    #[error(transparent)]
    Security(#[from] graphify_security::SecurityError),

    /// Claude CLI binary not found on `$PATH`.
    #[error(
        "Claude Code CLI not found on $PATH. Install from \
         https://claude.ai/code and run `claude` once to authenticate."
    )]
    ClaudeCliMissing,

    /// Claude CLI returned a non-zero exit code or unexpected output.
    #[error("{0}")]
    ClaudeCliError(String),

    /// Unknown backend name.
    #[error("Unknown backend {0:?}. Available: {1}")]
    UnknownBackend(String, String),
}

// ---------------------------------------------------------------------------
// Response type
// ---------------------------------------------------------------------------

/// Structured response from any LLM backend.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub nodes: Vec<serde_json::Value>,
    pub edges: Vec<serde_json::Value>,
    pub hyperedges: Vec<serde_json::Value>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model: String,
    /// `"stop"` or `"length"` (normalised from backend-specific values).
    pub finish_reason: String,
    /// Wall-clock time taken by this chunk, in seconds.
    pub elapsed_seconds: f64,
    /// Chunk indices (0-based) that failed during parallel extraction.
    pub failed_chunk_indices: Vec<usize>,
}

impl LlmResponse {
    /// Convert to a `serde_json::Value` map (matches Python dict shape).
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

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over LLM backends.
pub trait LlmBackend: Send + Sync {
    /// Backend identifier (`"claude"`, `"kimi"`, etc.).
    fn name(&self) -> &'static str;

    /// Send `messages` to the model and return a structured response.
    ///
    /// # Errors
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
