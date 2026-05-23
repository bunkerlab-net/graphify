//! Error type for LLM backend calls.

use thiserror::Error;

use graphify_security::SecurityError;

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
    Security(#[from] SecurityError),

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

    /// Caller-supplied input was rejected (e.g. zero token budget).
    #[error("Invalid input: {0}")]
    InvalidInput(String),
}
