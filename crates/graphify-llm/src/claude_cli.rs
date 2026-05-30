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
    ///
    /// `system_prompt` carries the extraction system prompt passed via
    /// `--system-prompt` (replacing Claude Code's default coding-agent prompt,
    /// per #1062); `None` passes no system prompt.
    fn run(&self, user_message: &str, system_prompt: Option<&str>) -> (String, String, i32);
}

/// Production runner that invokes the real `claude` binary.
pub struct RealClaudeRunner;

impl ClaudeRunner for RealClaudeRunner {
    /// Spawns the `claude -p` subprocess, writes `user_message` to stdin, and returns stdout/stderr/exit-code.
    fn run(&self, user_message: &str, system_prompt: Option<&str>) -> (String, String, i32) {
        let Some(program) = resolve_claude_command() else {
            return (
                String::new(),
                "Claude Code CLI not found on $PATH. Install from \
                 https://claude.ai/code and run `claude` once to authenticate."
                    .to_string(),
                1,
            );
        };
        let model = claude_cli_model_override();
        let args = build_claude_cli_args(system_prompt, model.as_deref());
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&args);
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

/// Check whether a usable `claude` binary is on `$PATH`.
#[must_use]
pub fn claude_is_on_path() -> bool {
    resolve_claude_command().is_some()
}

/// Searches `$PATH` for an executable named `name` and returns its path.
///
/// Mirrors `shutil.which`: a candidate must be a regular file and, on Unix, have
/// an executable bit set — a non-executable file of the same name is skipped.
#[must_use]
fn which_named(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(name);
            is_executable_file(&candidate).then_some(candidate)
        })
    })
}

/// `true` if `path` is a regular file with an executable bit set (Unix).
#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `true` if `path` is a regular file. On non-Unix platforms executability is
/// determined by extension (handled by [`select_claude_command`]), not a mode bit.
#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Pure resolution of the `claude` command to invoke, given the host platform
/// and a name→path lookup. Factored out so the Windows `claude.cmd` preference
/// (#1072) is testable without a real Windows host.
///
/// On Windows, npm installs `claude` as both `claude.ps1` and `claude.cmd`.
/// When `PATHEXT` lists `.PS1` before `.CMD`, a bare `claude` lookup resolves
/// to `claude.ps1`, which `CreateProcess` cannot execute directly (`WinError
/// 2`). `claude.cmd` IS executable, so its full path is preferred. Falls back
/// to the bare `claude` name when only that resolves (e.g. a WSL-style
/// install). Returns `None` when neither is present.
#[must_use]
pub fn select_claude_command(
    is_windows: bool,
    mut which: impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    if is_windows {
        if let Some(cmd_path) = which("claude.cmd") {
            return Some(cmd_path);
        }
        if which("claude").is_some() {
            return Some("claude".to_string());
        }
        return None;
    }
    which("claude").map(|_| "claude".to_string())
}

/// Resolve the real `claude` command for this host.
fn resolve_claude_command() -> Option<String> {
    select_claude_command(cfg!(windows), |name| {
        which_named(name).map(|p| p.to_string_lossy().into_owned())
    })
}

/// The `GRAPHIFY_CLAUDE_CLI_MODEL` override, trimmed; `None` when unset/blank.
///
/// `claude -p` defaults to Opus, which is overkill for structured-JSON
/// extraction. Setting `GRAPHIFY_CLAUDE_CLI_MODEL=haiku` (or `sonnet`, or a
/// full model id) opts into a cheaper/faster model.
fn claude_cli_model_override() -> Option<String> {
    let raw = std::env::var("GRAPHIFY_CLAUDE_CLI_MODEL").ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Build the `claude -p` argument vector (excluding the executable itself).
///
/// Mirrors graphify-py `_call_claude_cli`'s `cli_args`: uses `--system-prompt`
/// (replace) rather than `--append-system-prompt` (add) so graphify's
/// "raw JSON only" instruction does not conflict with Claude Code's default
/// markdown/prose coding-agent prompt (the root cause of the hollow-response
/// loop, #1062). Appends `--model` only when `model` is `Some` and non-empty.
#[must_use]
pub fn build_claude_cli_args(system_prompt: Option<&str>, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "json".to_string(),
        "--no-session-persistence".to_string(),
    ];
    if let Some(system) = system_prompt {
        args.push("--system-prompt".to_string());
        args.push(system.to_string());
    }
    if let Some(m) = model
        && !m.trim().is_empty()
    {
        args.push("--model".to_string());
        args.push(m.trim().to_string());
    }
    args
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
    call_claude_cli_with_runner_system(runner, user_message, max_tokens, EXTRACTION_SYSTEM)
}

/// Call the Claude CLI with the given runner and an explicit system prompt
/// (e.g. the deep-mode variant).
///
/// # Errors
/// Returns [`LlmError::ClaudeCliMissing`] when the binary isn't on `$PATH`, or
/// [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_with_runner_system(
    runner: &dyn ClaudeRunner,
    user_message: &str,
    max_tokens: u32,
    system: &str,
) -> Result<LlmResponse, LlmError> {
    if resolve_claude_command().is_none() {
        return Err(LlmError::ClaudeCliMissing);
    }
    call_claude_cli_inner(runner, user_message, max_tokens, Some(system))
}

/// Inner CLI call (extraction path — injects `system_prompt` when `Some`).
///
/// # Errors
/// Returns [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_inner(
    runner: &dyn ClaudeRunner,
    user_message: &str,
    _max_tokens: u32,
    system_prompt: Option<&str>,
) -> Result<LlmResponse, LlmError> {
    let (stdout, stderr, code) = runner.run(user_message, system_prompt);

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
    if resolve_claude_command().is_none() {
        return Err(LlmError::ClaudeCliMissing);
    }
    let runner = RealClaudeRunner;
    let (stdout, stderr, code) = runner.run(user_message, None);
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
