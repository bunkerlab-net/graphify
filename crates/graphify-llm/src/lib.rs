//! LLM backend abstraction for knowledge-graph extraction.
//!
//! Ports `graphify-py/graphify/llm.py`. Provides a unified interface for sending
//! extraction prompts to multiple LLM providers and merging the structured JSON
//! responses into a single [`LlmResponse`].
//!
//! # Backends
//!
//! Each backend wraps a concrete API:
//!
//! | Name | Provider | Auth env var(s) |
//! |------|----------|-----------------|
//! | `claude` | Anthropic Messages API | `ANTHROPIC_API_KEY` |
//! | `claude-cli` | Local `claude -p` binary | subscription / `claude auth` |
//! | `openai` | `OpenAI` Chat Completions | `OPENAI_API_KEY` |
//! | `gemini` | Google Generative Language | `GEMINI_API_KEY` / `GOOGLE_API_KEY` |
//! | `kimi` | Moonshot AI (Kimi K2) | `MOONSHOT_API_KEY` |
//! | `deepseek` | `DeepSeek` Chat Completions | `DEEPSEEK_API_KEY` |
//! | `ollama` | Local Ollama server | `OLLAMA_BASE_URL` (optional) |
//! | `bedrock` | AWS Bedrock Converse API (`aws-sdk-bedrockruntime`) | Any AWS credential provider: env vars, profile, SSO, IMDS, ECS, web identity |
//!
//! Use [`router`] to obtain a boxed [`LlmBackend`] by name, or call backend
//! functions directly for finer control.
//!
//! # High-level entry points
//!
//! - [`extract_files_direct`] — send a slice of files to one backend and return an [`LlmResponse`].
//! - [`extract_corpus_parallel`] — fan out across chunks with Rayon, then merge.
//! - [`extract_with_adaptive_retry`] — bisect oversized chunks on context-window errors.
//! - [`call_llm`] — send a plain-text prompt and receive a raw string reply.
//! - [`pack_chunks_by_tokens`] — group files into token-budget–sized chunks before extraction.

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
pub mod labeling;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod parallel;
pub mod parse;
pub mod providers;
pub mod read;
mod response;
pub mod retry;
pub mod tokenizer;
pub mod tokens;

pub use backends::{
    BACKENDS, BackendConfig, Pricing, backend_config, backend_selection_env_vars, detect_backend,
    detect_backend_with, format_backend_env_keys, get_backend_api_key, router,
};
pub use call::call_llm;
pub use constants::{
    DEEP_EXTRACTION_SUFFIX, EXTRACTION_SYSTEM, FILE_CHAR_CAP, LLM_JSON_MAX_BYTES,
    PER_FILE_OVERHEAD_CHARS, extraction_system,
};
pub use error::LlmError;
pub use extract::{extract_files_direct, extract_files_direct_mode};
pub use labeling::{
    generate_community_labels, generate_community_labels_with, label_communities,
    label_communities_with, placeholder_community_labels,
};
pub use parallel::{
    ChunkDoneCb, CorpusConfig, extract_corpus_parallel, extract_corpus_parallel_with_total,
    merge_into,
};
pub use parse::{empty_fragment, parse_llm_json, response_is_hollow};
pub use providers::{
    CustomProvider, custom_providers_path, is_builtin_backend, load_custom_providers,
    load_custom_providers_from,
};
pub use read::read_files;
pub use response::{LlmBackend, LlmResponse};
pub use retry::{
    extract_with_adaptive_retry, looks_like_context_exceeded, looks_like_context_exceeded_dyn,
};
pub use tokens::{estimate_cost, estimate_file_tokens, pack_chunks_by_tokens};
