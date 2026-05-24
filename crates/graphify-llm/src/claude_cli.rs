//! Claude CLI backend — shells out to the `claude -p` binary.
//!
//! Ports `_call_claude_cli` in `graphify-py/graphify/llm.py`.
//!
//! The `claude` binary is injected via the [`ClaudeRunner`] trait so tests
//! can substitute a mock without spawning a real process.

use serde_json::json;

use crate::{
    EXTRACTION_SYSTEM, LlmBackend, LlmError, LlmResponse, parse_llm_json, response_is_hollow,
};

/// Trait that abstracts the `claude -p` subprocess.
///
/// The real implementation calls the binary; tests inject a [`MockRunner`].
pub trait ClaudeRunner: Send + Sync {
    /// Run the claude CLI, return `(stdout, stderr, exit_code)`.
    fn run(&self, user_message: &str, append_system_prompt: bool) -> (String, String, i32);
}

/// Production runner that invokes the real `claude` binary.
pub struct RealClaudeRunner;

impl ClaudeRunner for RealClaudeRunner {
    /// Spawns the `claude -p` subprocess, writes `user_message` to stdin, and returns stdout/stderr/exit-code.
    fn run(&self, user_message: &str, append_system_prompt: bool) -> (String, String, i32) {
        let mut cmd = std::process::Command::new("claude");
        cmd.arg("-p")
            .arg("--output-format")
            .arg("json")
            .arg("--no-session-persistence");
        if append_system_prompt {
            cmd.arg("--append-system-prompt").arg(EXTRACTION_SYSTEM);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return (String::new(), e.to_string(), 1),
        };

        if let Some(stdin) = child.stdin.take() {
            use std::io::Write;
            let mut w = stdin;
            if let Err(e) = w.write_all(user_message.as_bytes()) {
                // Surface a write failure rather than letting `claude` see
                // an empty prompt; reap the child first so we don't leak it.
                let _ = child.kill();
                let _ = child.wait();
                return (
                    String::new(),
                    format!("write to claude stdin failed: {e}"),
                    1,
                );
            }
        }

        match child.wait_with_output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                let code = out.status.code().unwrap_or(1);
                (stdout, stderr, code)
            }
            Err(e) => (String::new(), e.to_string(), 1),
        }
    }
}

/// Claude CLI backend.
pub struct ClaudeCliBackend<R: ClaudeRunner = RealClaudeRunner> {
    runner: R,
}

impl ClaudeCliBackend<RealClaudeRunner> {
    /// Create using the real `claude` binary.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runner: RealClaudeRunner,
        }
    }
}

impl Default for ClaudeCliBackend<RealClaudeRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ClaudeRunner> ClaudeCliBackend<R> {
    /// Create with a custom runner (for testing).
    #[must_use]
    pub fn with_runner(runner: R) -> Self {
        Self { runner }
    }
}

impl<R: ClaudeRunner + 'static> LlmBackend for ClaudeCliBackend<R> {
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "claude-cli"
    }

    /// Extracts the last user message and calls the CLI runner via
    /// [`call_claude_cli_with_runner`].
    ///
    /// `model` and `max_tokens` are intentionally ignored: the `claude -p`
    /// CLI does not expose either as a flag, so the underlying runner uses
    /// its own subscription defaults.
    fn call(
        &self,
        messages: &[serde_json::Value],
        _model: &str,
        _max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        // Extract last user message content.
        let user_message = messages
            .iter()
            .filter_map(|m| {
                if m.get("role")?.as_str()? == "user" {
                    m.get("content")?.as_str()
                } else {
                    None
                }
            })
            .next_back()
            .unwrap_or("");

        call_claude_cli_with_runner(&self.runner, user_message, 8192)
    }

    /// Delegates to the shared tiktoken-based estimator.
    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Check whether the `claude` binary is on `$PATH`.
#[must_use]
pub fn claude_is_on_path() -> bool {
    which_claude().is_some()
}

/// Searches `$PATH` for a `claude` executable and returns its path if found.
fn which_claude() -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join("claude");
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

/// Call the Claude CLI with the given runner.
///
/// # Errors
/// Returns [`LlmError::ClaudeCliMissing`] when the binary isn't on `$PATH`, or
/// [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_with_runner(
    runner: &dyn ClaudeRunner,
    user_message: &str,
    max_tokens: u32,
) -> Result<LlmResponse, LlmError> {
    if which_claude().is_none() {
        return Err(LlmError::ClaudeCliMissing);
    }
    call_claude_cli_inner(runner, user_message, max_tokens, true)
}

/// Inner CLI call (extraction path — injects system prompt).
///
/// # Errors
/// Returns [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_inner(
    runner: &dyn ClaudeRunner,
    user_message: &str,
    _max_tokens: u32,
    append_system: bool,
) -> Result<LlmResponse, LlmError> {
    let (stdout, stderr, code) = runner.run(user_message, append_system);

    if code != 0 {
        let snippet = stderr.trim().chars().take(500).collect::<String>();
        return Err(LlmError::ClaudeCliError(format!(
            "claude -p exited {code}: {snippet}"
        )));
    }

    let envelope: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| {
        LlmError::ClaudeCliError(format!(
            "claude -p produced unparseable JSON envelope: {e}; \
             first 500 chars of stdout: {:?}",
            stdout.chars().take(500).collect::<String>()
        ))
    })?;

    let raw_content = envelope
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut parsed = parse_llm_json(&raw_content);

    let usage = envelope.get("usage").cloned().unwrap_or(json!({}));
    let input_tokens = usage
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        + usage
            .get("cache_read_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
        + usage
            .get("cache_creation_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    // Model: first key of modelUsage or default.
    let model = envelope
        .get("modelUsage")
        .and_then(|v| v.as_object())
        .and_then(|m| m.keys().next().cloned())
        .unwrap_or_else(|| "claude-code-plan".to_string());

    let stop_reason = envelope
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut finish_reason = if stop_reason == "max_tokens" {
        "length".to_string()
    } else {
        "stop".to_string()
    };

    if response_is_hollow(Some(&raw_content), &parsed) && finish_reason != "length" {
        eprintln!(
            "[graphify] claude-cli returned a hollow response; treating as \
             truncation so adaptive retry can bisect the chunk."
        );
        finish_reason = "length".to_string();
    }

    parsed["input_tokens"] = json!(input_tokens);
    parsed["output_tokens"] = json!(output_tokens);
    parsed["model"] = json!(&model);
    parsed["finish_reason"] = json!(&finish_reason);

    Ok(LlmResponse {
        nodes: parsed["nodes"].as_array().cloned().unwrap_or_default(),
        edges: parsed["edges"].as_array().cloned().unwrap_or_default(),
        hyperedges: parsed["hyperedges"].as_array().cloned().unwrap_or_default(),
        input_tokens,
        output_tokens,
        model,
        finish_reason,
        elapsed_seconds: 0.0,
        failed_chunk_indices: vec![],
    })
}

/// Call the Claude CLI for a plain-text response (no extraction system prompt).
///
/// Used by the LLM tiebreaker path.
///
/// # Errors
/// Returns [`LlmError::ClaudeCliMissing`] when the binary isn't on `$PATH`, or
/// [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_plain(user_message: &str, _max_tokens: u32) -> Result<String, LlmError> {
    if which_claude().is_none() {
        return Err(LlmError::ClaudeCliMissing);
    }
    let runner = RealClaudeRunner;
    let (stdout, stderr, code) = runner.run(user_message, false);
    if code != 0 {
        let snippet = stderr.trim().chars().take(500).collect::<String>();
        return Err(LlmError::ClaudeCliError(format!(
            "claude -p exited {code}: {snippet}"
        )));
    }
    let envelope: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| {
        LlmError::ClaudeCliError(format!("claude -p produced unparseable JSON envelope: {e}"))
    })?;
    Ok(envelope
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}
