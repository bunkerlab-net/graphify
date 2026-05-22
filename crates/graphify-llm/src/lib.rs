//! LLM router — ports `graphify-py/graphify/llm.py`.
//!
//! Provides:
//! - The [`LlmBackend`] trait with per-backend impls.
//! - A [`router`] factory to get a backend by name.
//! - Token estimation via [`tokenizer`].
//! - File packing, adaptive retry, and corpus-parallel extraction.

pub mod bedrock;
pub mod claude;
pub mod claude_cli;
pub mod deepseek;
pub mod gemini;
pub mod kimi;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod tokenizer;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
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
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub hyperedges: Vec<Value>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model: String,
    /// `"stop"` or `"length"` (normalised from backend-specific values).
    pub finish_reason: String,
}

impl LlmResponse {
    /// Convert to a `serde_json::Value` map (matches Python dict shape).
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
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
        messages: &[Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError>;

    /// Estimate token count for a text snippet.
    fn estimate_tokens(&self, text: &str) -> usize;
}

// ---------------------------------------------------------------------------
// Router / factory
// ---------------------------------------------------------------------------

/// Pricing entry (USD per 1M tokens).
#[derive(Debug, Clone, Copy)]
pub struct Pricing {
    pub input: f64,
    pub output: f64,
}

/// Static backend metadata (mirrors Python `BACKENDS` dict).
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub name: &'static str,
    pub default_model: &'static str,
    pub pricing: Pricing,
    pub default_max_tokens: u32,
}

/// All registered backends.
pub const BACKENDS: &[BackendConfig] = &[
    BackendConfig {
        name: "claude",
        default_model: claude::DEFAULT_MODEL,
        pricing: Pricing {
            input: 3.0,
            output: 15.0,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "kimi",
        default_model: kimi::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.74,
            output: 4.66,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "gemini",
        default_model: gemini::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.50,
            output: 3.00,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "openai",
        default_model: openai::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.40,
            output: 1.60,
        },
        default_max_tokens: 8_192,
    },
    BackendConfig {
        name: "deepseek",
        default_model: deepseek::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.14,
            output: 0.28,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "ollama",
        default_model: ollama::DEFAULT_MODEL,
        pricing: Pricing {
            input: 0.0,
            output: 0.0,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "bedrock",
        default_model: bedrock::DEFAULT_MODEL,
        pricing: Pricing {
            input: 3.0,
            output: 15.0,
        },
        default_max_tokens: 16_384,
    },
    BackendConfig {
        name: "claude-cli",
        default_model: "claude-code-plan",
        pricing: Pricing {
            input: 0.0,
            output: 0.0,
        },
        default_max_tokens: 16_384,
    },
];

/// Look up a backend config by name.
#[must_use]
pub fn backend_config(name: &str) -> Option<&'static BackendConfig> {
    BACKENDS.iter().find(|b| b.name == name)
}

/// Construct a boxed [`LlmBackend`] by name.
///
/// # Errors
/// Returns [`LlmError::UnknownBackend`] if `name` is not registered.
pub fn router(name: &str) -> Result<Box<dyn LlmBackend>, LlmError> {
    match name {
        "claude" => Ok(Box::new(claude::ClaudeBackend::from_env())),
        "kimi" => Ok(Box::new(kimi::KimiBackend::from_env())),
        "gemini" => Ok(Box::new(gemini::GeminiBackend::from_env())),
        "openai" => Ok(Box::new(openai::OpenAiBackend::from_env())),
        "deepseek" => Ok(Box::new(deepseek::DeepSeekBackend::from_env())),
        "ollama" => Ok(Box::new(ollama::OllamaBackend::from_env())),
        "bedrock" => Ok(Box::new(bedrock::BedrockBackend::from_env())),
        "claude-cli" => Ok(Box::new(claude_cli::ClaudeCliBackend::new())),
        other => {
            let available = BACKENDS
                .iter()
                .map(|b| b.name)
                .collect::<Vec<_>>()
                .join(", ");
            Err(LlmError::UnknownBackend(other.to_string(), available))
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: get API key for a backend
// ---------------------------------------------------------------------------

/// Return the first available API key for the named backend (or empty string).
#[must_use]
pub fn get_backend_api_key(backend: &str) -> String {
    match backend {
        "gemini" => gemini::get_api_key(),
        "kimi" => std::env::var(kimi::ENV_KEY).unwrap_or_default(),
        "claude" => std::env::var(claude::ENV_KEY).unwrap_or_default(),
        "openai" => std::env::var(openai::ENV_KEY).unwrap_or_default(),
        "deepseek" => std::env::var(deepseek::ENV_KEY).unwrap_or_default(),
        "ollama" => std::env::var(ollama::ENV_KEY).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Return user-facing env var names for the backend.
#[must_use]
pub fn format_backend_env_keys(backend: &str) -> String {
    match backend {
        "gemini" => format!("{} or {}", gemini::ENV_KEY, gemini::ENV_KEY_FALLBACK),
        "kimi" => kimi::ENV_KEY.to_string(),
        "claude" => claude::ENV_KEY.to_string(),
        "openai" => openai::ENV_KEY.to_string(),
        "deepseek" => deepseek::ENV_KEY.to_string(),
        "ollama" => ollama::ENV_KEY.to_string(),
        _ => "AWS_PROFILE or AWS_REGION".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Backend auto-detection
// ---------------------------------------------------------------------------

/// Detect which backend has a key configured.
///
/// Priority: gemini → kimi → claude → openai → deepseek → bedrock → ollama.
/// Returns `None` if no backend is configured.
#[must_use]
pub fn detect_backend() -> Option<String> {
    for backend in ["gemini", "kimi", "claude", "openai", "deepseek"] {
        if !get_backend_api_key(backend).is_empty() {
            return Some(backend.to_string());
        }
    }
    // Bedrock: check for any AWS env var.
    if std::env::var("AWS_PROFILE").is_ok()
        || std::env::var("AWS_REGION").is_ok()
        || std::env::var("AWS_DEFAULT_REGION").is_ok()
    {
        return Some("bedrock".to_string());
    }
    // Ollama: checked last to avoid shadowing paid backends.
    if let Ok(url) = std::env::var("OLLAMA_BASE_URL")
        && !url.is_empty()
    {
        ollama::validate_ollama_base_url(&url);
        return Some("ollama".to_string());
    }
    None
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

/// Return an empty extraction fragment.
#[must_use]
pub fn empty_fragment() -> Value {
    json!({"nodes": [], "edges": [], "hyperedges": []})
}

/// Strip optional markdown fences and parse JSON.
///
/// Returns an empty fragment on failure. Capped at [`LLM_JSON_MAX_BYTES`].
#[must_use]
pub fn parse_llm_json(raw: &str) -> Value {
    if raw.len() > LLM_JSON_MAX_BYTES {
        eprintln!(
            "[graphify] LLM response exceeds {LLM_JSON_MAX_BYTES} bytes \
             ({} bytes); refusing to parse and dropping chunk.",
            raw.len()
        );
        return empty_fragment();
    }
    let mut s = raw.trim();
    if s.starts_with("```") {
        let parts: Vec<&str> = s.splitn(3, "```").collect();
        if parts.len() >= 2 {
            let mut inner = parts[1];
            if inner.starts_with("json") {
                inner = &inner[4..];
            }
            // Strip trailing fence
            let trimmed = inner.trim();
            if let Some(idx) = trimmed.rfind("```") {
                s = trimmed[..idx].trim();
            } else {
                s = trimmed;
            }
        }
    }
    match serde_json::from_str::<Value>(s) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[graphify] LLM returned invalid JSON, skipping chunk: {e}");
            empty_fragment()
        }
    }
}

/// Return `true` if the response produced no usable nodes, edges, or hyperedges.
#[must_use]
pub fn response_is_hollow(raw_content: Option<&str>, parsed: &Value) -> bool {
    match raw_content {
        None => return true,
        Some(s) if s.trim().is_empty() => return true,
        Some(_) => {}
    }
    let is_empty_arr = |key: &str| {
        parsed
            .get(key)
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    };
    is_empty_arr("nodes") && is_empty_arr("edges") && is_empty_arr("hyperedges")
}

// ---------------------------------------------------------------------------
// File reading
// ---------------------------------------------------------------------------

/// Read and format file contents for the extraction prompt.
///
/// Each file is capped at [`FILE_CHAR_CAP`] chars and wrapped in
/// `=== {rel} ===\n{content}` sections separated by blank lines.
#[must_use]
pub fn read_files(paths: &[PathBuf], root: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in paths {
        let rel = p.strip_prefix(root).unwrap_or(p.as_path());
        let Ok(content) = std::fs::read_to_string(p) else {
            continue;
        };
        let capped: String = content.chars().take(FILE_CHAR_CAP).collect();
        parts.push(format!("=== {} ===\n{capped}", rel.display()));
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Token estimation for file packing
// ---------------------------------------------------------------------------

/// Estimate the token cost of one file under `read_files` rules.
#[must_use]
pub fn estimate_file_tokens(path: &Path) -> usize {
    if let Ok(content) = std::fs::read_to_string(path) {
        let capped: String = content.chars().take(FILE_CHAR_CAP).collect();
        tokenizer::estimate_file_tokens(&capped, PER_FILE_OVERHEAD_CHARS)
    } else {
        // Fallback: use file size.
        let size = path
            .metadata()
            .map_or(0, |m| usize::try_from(m.len()).unwrap_or(usize::MAX));
        let chars = size.min(FILE_CHAR_CAP) + PER_FILE_OVERHEAD_CHARS;
        chars / tokenizer::CHARS_PER_TOKEN
    }
}

// ---------------------------------------------------------------------------
// Chunk packing
// ---------------------------------------------------------------------------

/// Pack files into token-budget chunks, grouped by parent directory.
///
/// # Errors
/// Returns an error if `token_budget` is zero.
pub fn pack_chunks_by_tokens(
    files: &[PathBuf],
    token_budget: usize,
) -> Result<Vec<Vec<PathBuf>>, LlmError> {
    if token_budget == 0 {
        return Err(LlmError::Http("token_budget must be positive".to_string()));
    }

    // Group by parent directory (preserving order).
    let mut by_dir: indexmap::IndexMap<PathBuf, Vec<PathBuf>> = indexmap::IndexMap::new();
    for f in files {
        let parent = f.parent().unwrap_or(Path::new(".")).to_path_buf();
        by_dir.entry(parent).or_default().push(f.clone());
    }

    // Sort directories for deterministic output.
    by_dir.sort_keys();

    let mut chunks: Vec<Vec<PathBuf>> = Vec::new();
    let mut current: Vec<PathBuf> = Vec::new();
    let mut current_tokens: usize = 0;

    for (_dir, dir_files) in &by_dir {
        for path in dir_files {
            let cost = estimate_file_tokens(path);
            if !current.is_empty() && current_tokens + cost > token_budget {
                chunks.push(std::mem::take(&mut current));
                current_tokens = 0;
            }
            current.push(path.clone());
            current_tokens += cost;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

// ---------------------------------------------------------------------------
// Context-exceeded detection
// ---------------------------------------------------------------------------

const CONTEXT_EXCEEDED_MARKERS: &[&str] = &[
    "context size",
    "context length",
    "context_length",
    "context window",
    "n_keep",
    "exceeds the available",
    "n_ctx",
    "maximum context",
    "too many tokens",
    "prompt is too long",
    "context_length_exceeded",
];

/// Heuristically classify an error as context-window overflow.
#[must_use]
pub fn looks_like_context_exceeded(err: &LlmError) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    CONTEXT_EXCEEDED_MARKERS.iter().any(|m| msg.contains(m))
}

/// Same check against a boxed `std::error::Error`.
#[must_use]
pub fn looks_like_context_exceeded_dyn(err: &(dyn std::error::Error + Send + Sync)) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    CONTEXT_EXCEEDED_MARKERS.iter().any(|m| msg.contains(m))
}

// ---------------------------------------------------------------------------
// Cost estimation
// ---------------------------------------------------------------------------

/// Estimate USD cost for a given token count.
///
/// Returns 0.0 for unknown backends.
#[must_use]
pub fn estimate_cost(backend: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let Some(cfg) = backend_config(backend) else {
        return 0.0;
    };
    // Allow precision loss: token counts are at most ~10^9, well within f64 range.
    #[allow(clippy::cast_precision_loss)]
    let cost = (input_tokens as f64 * cfg.pricing.input
        + output_tokens as f64 * cfg.pricing.output)
        / 1_000_000.0;
    cost
}

// ---------------------------------------------------------------------------
// Extract files directly
// ---------------------------------------------------------------------------

/// Extract semantic nodes/edges from a list of files using the given backend.
///
/// # Errors
/// - [`LlmError::UnknownBackend`] for unregistered backend names.
/// - [`LlmError::NoApiKey`] when no key is configured (except `bedrock`/`claude-cli`).
/// - Backend-specific HTTP / parse errors.
pub fn extract_files_direct(
    files: &[PathBuf],
    backend: &str,
    api_key: Option<&str>,
    model: Option<&str>,
    root: &Path,
) -> Result<LlmResponse, LlmError> {
    let cfg = backend_config(backend).ok_or_else(|| {
        let available = BACKENDS
            .iter()
            .map(|b| b.name)
            .collect::<Vec<_>>()
            .join(", ");
        LlmError::UnknownBackend(backend.to_string(), available)
    })?;

    let key = api_key
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| get_backend_api_key(backend));

    // Ollama: use sentinel "ollama" key when none configured.
    let key = if key.is_empty() && backend == "ollama" {
        let ollama_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        ollama::validate_ollama_base_url(&ollama_url);
        eprintln!(
            "[graphify] WARNING: ollama backend selected with no OLLAMA_API_KEY set; \
             sending corpus to {ollama_url}. Set OLLAMA_API_KEY (any non-empty value) \
             to suppress this warning."
        );
        "ollama".to_string()
    } else {
        key
    };

    if key.is_empty() && backend != "bedrock" && backend != "claude-cli" {
        return Err(LlmError::NoApiKey(format!(
            "No API key for backend '{backend}'. \
             Set {} or pass api_key=.",
            format_backend_env_keys(backend)
        )));
    }

    let mdl = model.filter(|s| !s.is_empty()).unwrap_or(cfg.default_model);
    let user_msg = read_files(files, root);
    let max_out = openai_compat::resolve_max_tokens(cfg.default_max_tokens);

    match backend {
        "claude" => {
            let msgs = vec![json!({"role": "user", "content": user_msg})];
            claude::call_claude(&key, mdl, &msgs, max_out)
        }
        "claude-cli" => {
            let runner = claude_cli::RealClaudeRunner;
            claude_cli::call_claude_cli_with_runner(&runner, &user_msg, max_out)
        }
        "bedrock" => {
            let region = bedrock::resolve_region();
            let msgs = vec![json!({"role": "user", "content": [{"text": user_msg}]})];
            bedrock::call_bedrock(mdl, &region, &msgs, max_out)
        }
        "kimi" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            kimi::call_kimi(&key, mdl, &msgs, max_out)
        }
        "gemini" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            gemini::call_gemini(&key, mdl, &msgs, max_out)
        }
        "openai" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            openai::call_openai(&key, mdl, &msgs, max_out)
        }
        "deepseek" => {
            let msgs = openai_compat::extraction_messages(&user_msg);
            deepseek::call_deepseek(&key, mdl, &msgs, max_out)
        }
        "ollama" => {
            let base_url = std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
            let msgs = openai_compat::extraction_messages(&user_msg);
            ollama::call_ollama(&key, &base_url, mdl, &msgs, max_out, &user_msg)
        }
        _ => unreachable!("backend_config already validated backend name"),
    }
}

// ---------------------------------------------------------------------------
// Adaptive retry
// ---------------------------------------------------------------------------

/// Extract a chunk; split in half and retry on context overflow or truncation.
///
/// # Errors
/// Propagates errors that don't look like context-window overflow.
pub fn extract_with_adaptive_retry(
    chunk: &[PathBuf],
    backend: &str,
    api_key: Option<&str>,
    model: Option<&str>,
    root: &Path,
    max_depth: usize,
    depth: usize,
) -> Result<LlmResponse, LlmError> {
    let result = extract_files_direct(chunk, backend, api_key, model, root);

    match result {
        Err(ref e) if looks_like_context_exceeded(e) => {
            if chunk.len() <= 1 {
                eprintln!(
                    "[graphify] single-file chunk {} exceeds model context \
                     and cannot be split further: {e}",
                    chunk
                        .first()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                return Ok(empty_llm_response(model));
            }
            if depth >= max_depth {
                eprintln!(
                    "[graphify] chunk of {} still overflows context at recursion \
                     depth {depth} (max {max_depth}) — dropping",
                    chunk.len()
                );
                return Ok(empty_llm_response(model));
            }
            eprintln!(
                "[graphify] chunk of {} exceeded context at depth {depth} \
                 (context overflow); splitting in half and retrying",
                chunk.len()
            );
            let mid = chunk.len() / 2;
            let left = extract_with_adaptive_retry(
                &chunk[..mid],
                backend,
                api_key,
                model,
                root,
                max_depth,
                depth + 1,
            )?;
            let right = extract_with_adaptive_retry(
                &chunk[mid..],
                backend,
                api_key,
                model,
                root,
                max_depth,
                depth + 1,
            )?;
            Ok(merge_responses(&left, &right, model))
        }
        Err(e) => Err(e),
        Ok(resp) if resp.finish_reason == "length" => {
            if chunk.len() <= 1 {
                eprintln!(
                    "[graphify] single-file chunk {} truncated at \
                     max_completion_tokens — partial result kept",
                    chunk
                        .first()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                return Ok(resp);
            }
            if depth >= max_depth {
                eprintln!(
                    "[graphify] chunk of {} still truncated at recursion depth {depth} \
                     (max {max_depth}) — partial result kept",
                    chunk.len()
                );
                return Ok(resp);
            }
            eprintln!(
                "[graphify] chunk of {} truncated at depth {depth}, \
                 splitting into halves of {} and {}",
                chunk.len(),
                chunk.len() / 2,
                chunk.len() - chunk.len() / 2,
            );
            let mid = chunk.len() / 2;
            let left = extract_with_adaptive_retry(
                &chunk[..mid],
                backend,
                api_key,
                model,
                root,
                max_depth,
                depth + 1,
            )?;
            let right = extract_with_adaptive_retry(
                &chunk[mid..],
                backend,
                api_key,
                model,
                root,
                max_depth,
                depth + 1,
            )?;
            Ok(merge_responses(&left, &right, model))
        }
        Ok(resp) => Ok(resp),
    }
}

fn empty_llm_response(model: Option<&str>) -> LlmResponse {
    LlmResponse {
        nodes: vec![],
        edges: vec![],
        hyperedges: vec![],
        input_tokens: 0,
        output_tokens: 0,
        model: model.unwrap_or("").to_string(),
        finish_reason: "stop".to_string(),
    }
}

fn merge_responses(left: &LlmResponse, right: &LlmResponse, model: Option<&str>) -> LlmResponse {
    let mut nodes = left.nodes.clone();
    nodes.extend_from_slice(&right.nodes);
    let mut edges = left.edges.clone();
    edges.extend_from_slice(&right.edges);
    let mut hyperedges = left.hyperedges.clone();
    hyperedges.extend_from_slice(&right.hyperedges);
    LlmResponse {
        nodes,
        edges,
        hyperedges,
        input_tokens: left.input_tokens + right.input_tokens,
        output_tokens: left.output_tokens + right.output_tokens,
        model: model.map_or_else(|| left.model.clone(), str::to_string),
        finish_reason: "stop".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Corpus parallel extraction (single-threaded variant for now)
// ---------------------------------------------------------------------------

/// Merge a chunk result into the running accumulator.
pub fn merge_into(merged: &mut LlmResponse, result: &LlmResponse) {
    merged.nodes.extend_from_slice(&result.nodes);
    merged.edges.extend_from_slice(&result.edges);
    merged.hyperedges.extend_from_slice(&result.hyperedges);
    merged.input_tokens += result.input_tokens;
    merged.output_tokens += result.output_tokens;
}

/// Configuration for [`extract_corpus_parallel`].
pub struct CorpusConfig<'a> {
    pub backend: &'a str,
    pub api_key: Option<&'a str>,
    pub model: Option<&'a str>,
    pub root: &'a Path,
    pub chunk_size: usize,
    pub token_budget: Option<usize>,
    pub max_concurrency: usize,
    pub max_retry_depth: usize,
}

/// Callback type for chunk-done notifications.
pub type ChunkDoneCb = dyn Fn(usize, usize, &LlmResponse);

/// Extract a corpus in chunks, merging results.
///
/// Uses thread-pool concurrency when `max_concurrency > 1` and the backend
/// allows it (ollama and claude-cli are forced serial by default).
///
/// Returns the merged response and a count of failed chunks.
pub fn extract_corpus_parallel(
    files: &[PathBuf],
    cfg: &CorpusConfig<'_>,
    on_chunk_done: Option<&ChunkDoneCb>,
) -> (LlmResponse, usize) {
    let chunks: Vec<Vec<PathBuf>> = if let Some(budget) = cfg.token_budget {
        pack_chunks_by_tokens(files, budget).unwrap_or_else(|_| {
            files
                .chunks(cfg.chunk_size.max(1))
                .map(<[PathBuf]>::to_vec)
                .collect()
        })
    } else {
        files
            .chunks(cfg.chunk_size.max(1))
            .map(<[PathBuf]>::to_vec)
            .collect()
    };

    let total = chunks.len();

    // Force serial for backends that don't support concurrent calls.
    let force_serial = (cfg.backend == "ollama"
        && std::env::var("GRAPHIFY_OLLAMA_PARALLEL")
            .as_deref()
            .unwrap_or("")
            .trim()
            != "1")
        || (cfg.backend == "claude-cli"
            && std::env::var("GRAPHIFY_CLAUDE_CLI_PARALLEL")
                .as_deref()
                .unwrap_or("")
                .trim()
                != "1");
    let _workers = if force_serial {
        1_usize
    } else {
        cfg.max_concurrency.max(1).min(total.max(1))
    };

    let mut merged = LlmResponse {
        nodes: vec![],
        edges: vec![],
        hyperedges: vec![],
        input_tokens: 0,
        output_tokens: 0,
        model: cfg.model.unwrap_or("").to_string(),
        finish_reason: "stop".to_string(),
    };
    let mut failed_chunks: usize = 0;

    for (idx, chunk) in chunks.iter().enumerate() {
        match extract_with_adaptive_retry(
            chunk,
            cfg.backend,
            cfg.api_key,
            cfg.model,
            cfg.root,
            cfg.max_retry_depth,
            0,
        ) {
            Ok(result) => {
                merge_into(&mut merged, &result);
                if let Some(cb) = on_chunk_done {
                    cb(idx, total, &result);
                }
            }
            Err(e) => {
                eprintln!("[graphify] chunk {}/{total} failed: {e}", idx + 1);
                failed_chunks += 1;
            }
        }
    }

    if failed_chunks > 0 {
        eprintln!(
            "[graphify] WARNING: {failed_chunks}/{total} semantic chunk(s) failed \
             — see errors above. Partial results returned."
        );
    }

    (merged, failed_chunks)
}
