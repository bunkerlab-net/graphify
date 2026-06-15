//! Claude CLI backend — shells out to the `claude -p` binary.
//!
//! Ports `_call_claude_cli` in `graphify-py/graphify/llm.py`.
//!
//! The `claude` binary is injected via the [`ClaudeRunner`] trait so tests
//! can substitute a mock without spawning a real process.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use wait_timeout::ChildExt;

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
    /// per #1062); `None` passes no system prompt. `model` overrides the CLI's
    /// default model via `--model` when `Some` (#b304331); `None` falls back to
    /// the `GRAPHIFY_CLAUDE_CLI_MODEL` env override. `timeout` bounds the
    /// subprocess wall-clock (`GRAPHIFY_API_TIMEOUT`, #1112); the process is
    /// killed and a timeout error returned if it is exceeded.
    fn run(
        &self,
        user_message: &str,
        system_prompt: Option<&str>,
        model: Option<&str>,
        add_dirs: &[PathBuf],
        timeout: Duration,
    ) -> (String, String, i32);
}

/// Production runner that invokes the real `claude` binary.
pub struct RealClaudeRunner;

impl ClaudeRunner for RealClaudeRunner {
    /// Spawns the `claude -p` subprocess, writes `user_message` to stdin, drains
    /// stdout/stderr on background threads (so a full pipe can't deadlock the
    /// wait), and bounds the wait by `timeout`. Returns stdout/stderr/exit-code;
    /// on timeout the child is killed and a non-zero code with a timeout message
    /// is returned. Mirrors graphify-py's `subprocess.run(..., timeout=…)`.
    fn run(
        &self,
        user_message: &str,
        system_prompt: Option<&str>,
        model: Option<&str>,
        add_dirs: &[PathBuf],
        timeout: Duration,
    ) -> (String, String, i32) {
        let Some(program) = resolve_claude_command() else {
            return (
                String::new(),
                "Claude Code CLI not found on $PATH. Install from \
                 https://claude.ai/code and run `claude` once to authenticate."
                    .to_string(),
                1,
            );
        };
        // Explicit per-call override wins; otherwise fall back to the env knob.
        let model = model
            .map(str::to_string)
            .filter(|m| !m.trim().is_empty())
            .or_else(claude_cli_model_override);
        let args = build_claude_cli_args(system_prompt, model.as_deref(), add_dirs);
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&args);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        apply_no_window(&mut cmd);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return (String::new(), e.to_string(), 1),
        };

        // Feed stdin on a thread: a large prompt could otherwise block the
        // write before `claude` starts reading, deadlocking against our own
        // reads below. Dropping the writer closes stdin (EOF) when done.
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let msg = user_message.as_bytes().to_vec();
            std::thread::spawn(move || {
                let _ = stdin.write_all(&msg);
            });
        }

        // Drain stdout/stderr concurrently so a full OS pipe buffer can't wedge
        // the process while we wait.
        let stdout_handle = child.stdout.take().map(spawn_reader);
        let stderr_handle = child.stderr.take().map(spawn_reader);

        let status = match child.wait_timeout(timeout) {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // Drain the reader threads so they don't leak before returning.
                drop(stdout_handle.and_then(|h| h.join().ok()));
                drop(stderr_handle.and_then(|h| h.join().ok()));
                return (
                    String::new(),
                    format!("claude -p timed out after {:.0}s", timeout.as_secs_f64()),
                    1,
                );
            }
            Err(e) => {
                // OS-level wait failure: reap the child and drain the reader
                // threads before returning, mirroring the timeout branch above
                // so a failed wait never leaks the subprocess or its pipe
                // readers.
                let _ = child.kill();
                let _ = child.wait();
                drop(stdout_handle.and_then(|h| h.join().ok()));
                drop(stderr_handle.and_then(|h| h.join().ok()));
                return (String::new(), e.to_string(), 1);
            }
        };

        let stdout = stdout_handle
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        let stderr = stderr_handle
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        let code = status.code().unwrap_or(1);
        (stdout, stderr, code)
    }
}

/// Suppress the console window the npm `claude.cmd` shim would otherwise pop
/// per spawn on Windows (#96585ba). `CREATE_NO_WINDOW` keeps the children
/// invisible during labeling/extraction runs; a no-op on other platforms.
#[cfg(windows)]
fn apply_no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// No-op on non-Windows platforms (no detached-console concept).
#[cfg(not(windows))]
fn apply_no_window(_cmd: &mut std::process::Command) {}

/// Spawn a thread that reads a child pipe to EOF as a lossy UTF-8 `String`.
fn spawn_reader<R: std::io::Read + Send + 'static>(
    mut reader: R,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
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
pub fn build_claude_cli_args(
    system_prompt: Option<&str>,
    model: Option<&str>,
    add_dirs: &[PathBuf],
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "json".to_string(),
        "--no-session-persistence".to_string(),
    ];
    // Allowlist each image directory so the Read tool can open the files
    // (#1110). The dirs are pre-deduplicated by the caller.
    for dir in add_dirs {
        args.push("--add-dir".to_string());
        args.push(dir.to_string_lossy().into_owned());
    }
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
    call_claude_cli_with_runner_system(runner, user_message, max_tokens, EXTRACTION_SYSTEM, &[])
}

/// Call the Claude CLI with the given runner and an explicit system prompt
/// (e.g. the deep-mode variant).
///
/// `images` carries raster-image refs (#1110): claude-cli reads images by path
/// rather than inline base64, so the prompt is appended with the Read-the-path
/// notes (`with_paths = true`) and each containing directory is allowlisted via
/// `--add-dir` so the Read tool can open it. Pass `&[]` when there are no images.
///
/// # Errors
/// Returns [`LlmError::ClaudeCliMissing`] when the binary isn't on `$PATH`, or
/// [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_with_runner_system(
    runner: &dyn ClaudeRunner,
    user_message: &str,
    max_tokens: u32,
    system: &str,
    images: &[crate::vision::ImageRef],
) -> Result<LlmResponse, LlmError> {
    if resolve_claude_command().is_none() {
        return Err(LlmError::ClaudeCliMissing);
    }
    let message = crate::vision::with_image_notes(user_message, images, true);
    let add_dirs = image_parent_dirs(images);
    cli_inner_with_dirs(runner, &message, max_tokens, Some(system), None, &add_dirs)
}

/// Deduplicated parent directories of the image paths, preserving first-seen
/// order (mirrors graphify-py's `seen_dirs` allowlist build).
fn image_parent_dirs(images: &[crate::vision::ImageRef]) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for img in images {
        if let Some(parent) = img.path.parent() {
            let dir = parent.to_path_buf();
            if !seen.contains(&dir) {
                seen.push(dir);
            }
        }
    }
    seen
}

/// Inner CLI call (extraction path — injects `system_prompt` when `Some`).
///
/// `model` overrides the CLI's default model via `--model` when `Some`; the
/// extraction path passes `None` and relies on the `GRAPHIFY_CLAUDE_CLI_MODEL`
/// env override resolved inside the runner.
///
/// # Errors
/// Returns [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_inner(
    runner: &dyn ClaudeRunner,
    user_message: &str,
    max_tokens: u32,
    system_prompt: Option<&str>,
    model: Option<&str>,
) -> Result<LlmResponse, LlmError> {
    cli_inner_with_dirs(runner, user_message, max_tokens, system_prompt, model, &[])
}

/// Inner CLI call with image directories allowlisted via `--add-dir` (#1110).
/// `call_claude_cli_inner` is the `add_dirs = &[]` case.
///
/// # Errors
/// Returns [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
fn cli_inner_with_dirs(
    runner: &dyn ClaudeRunner,
    user_message: &str,
    _max_tokens: u32,
    system_prompt: Option<&str>,
    model: Option<&str>,
    add_dirs: &[PathBuf],
) -> Result<LlmResponse, LlmError> {
    let (stdout, stderr, code) = runner.run(
        user_message,
        system_prompt,
        model,
        add_dirs,
        crate::openai_compat::api_timeout(),
    );

    if code != 0 {
        let snippet = stderr.trim().chars().take(500).collect::<String>();
        return Err(LlmError::ClaudeCliError(format!(
            "claude -p exited {code}: {snippet}"
        )));
    }

    let envelope = claude_cli_envelope(&stdout)?;

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
pub fn call_claude_cli_plain(user_message: &str, max_tokens: u32) -> Result<String, LlmError> {
    call_claude_cli_plain_with_model(user_message, max_tokens, None)
}

/// [`call_claude_cli_plain`] with an optional `--model` override (#b304331).
///
/// # Errors
/// Returns [`LlmError::ClaudeCliMissing`] when the binary isn't on `$PATH`, or
/// [`LlmError::ClaudeCliError`] on non-zero exit or unparseable output.
pub fn call_claude_cli_plain_with_model(
    user_message: &str,
    _max_tokens: u32,
    model: Option<&str>,
) -> Result<String, LlmError> {
    if resolve_claude_command().is_none() {
        return Err(LlmError::ClaudeCliMissing);
    }
    let runner = RealClaudeRunner;
    let (stdout, stderr, code) = runner.run(
        user_message,
        None,
        model,
        &[],
        crate::openai_compat::api_timeout(),
    );
    if code != 0 {
        let snippet = stderr.trim().chars().take(500).collect::<String>();
        return Err(LlmError::ClaudeCliError(format!(
            "claude -p exited {code}: {snippet}"
        )));
    }
    let envelope = claude_cli_envelope(&stdout)?;
    Ok(envelope
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// Parse the JSON from `claude -p --output-format json`, normalizing both
/// envelope shapes (#edfe581). Older CLIs returned a single envelope object;
/// CLI >= ~2.1 emits a JSON ARRAY of streamed event objects (system init,
/// assistant turns, optional rate-limit event, and a terminal
/// `{"type":"result"}`). Returns the result dict either way.
fn claude_cli_envelope(stdout: &str) -> Result<serde_json::Value, LlmError> {
    let snippet = || stdout.chars().take(500).collect::<String>();
    let parsed: serde_json::Value = serde_json::from_str(stdout).map_err(|e| {
        LlmError::ClaudeCliError(format!(
            "claude -p produced unparseable JSON envelope: {e}; \
             first 500 chars of stdout: {:?}",
            snippet()
        ))
    })?;
    let Some(events) = parsed.as_array() else {
        return Ok(parsed);
    };
    // Prefer the terminal {"type":"result"} event; fall back to the last
    // object in the stream.
    if let Some(result) = events
        .iter()
        .rev()
        .find(|e| e.get("type").and_then(serde_json::Value::as_str) == Some("result"))
    {
        return Ok(result.clone());
    }
    if let Some(last) = events.last().filter(|e| e.is_object()) {
        return Ok(last.clone());
    }
    Err(LlmError::ClaudeCliError(format!(
        "claude -p returned a JSON array with no result object; \
         first 500 chars of stdout: {:?}",
        snippet()
    )))
}
