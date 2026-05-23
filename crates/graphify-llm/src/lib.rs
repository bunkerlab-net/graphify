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
mod constants;
pub mod deepseek;
mod error;
pub mod extract;
pub mod gemini;
pub mod kimi;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod parallel;
pub mod parse;
pub mod read;
mod response;
pub mod retry;
pub mod tokenizer;
pub mod tokens;

pub use backends::{
    BACKENDS, BackendConfig, Pricing, backend_config, detect_backend, format_backend_env_keys,
    get_backend_api_key, router,
};
pub use call::call_llm;
pub use constants::{
    EXTRACTION_SYSTEM, FILE_CHAR_CAP, LLM_JSON_MAX_BYTES, PER_FILE_OVERHEAD_CHARS,
};
pub use error::LlmError;
pub use extract::extract_files_direct;
pub use parallel::{ChunkDoneCb, CorpusConfig, extract_corpus_parallel, merge_into};
pub use parse::{empty_fragment, parse_llm_json, response_is_hollow};
pub use read::read_files;
pub use response::{LlmBackend, LlmResponse};
pub use retry::{
    extract_with_adaptive_retry, looks_like_context_exceeded, looks_like_context_exceeded_dyn,
};
pub use tokens::{estimate_cost, estimate_file_tokens, pack_chunks_by_tokens};
