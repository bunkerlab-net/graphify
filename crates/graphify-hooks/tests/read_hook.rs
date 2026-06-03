//! Parity port of `graphify-py/tests/test_read_hook.py`.
//!
//! The `Read|Glob` `PreToolUse` hook nudges toward the graph instead of raw
//! reads (#1114): the Bash search hook never sees a file read through the Read
//! tool or a Glob. These tests run the hook command the way Claude Code does —
//! via `sh -c` with crafted stdin JSON — and assert it nudges only for a
//! source/doc file outside `graphify-out/` when a graph exists, otherwise stays
//! silent and fails open.
//!
//! The command is read back out of the settings.json that `claude_install`
//! writes, so the test exercises the real installed artifact.

#![allow(clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Stdio};

use graphify_hooks::platform::claude_install;
use serde_json::{Value, json};

/// Install graphify and return the `Read|Glob` hook command from settings.json.
fn read_hook_command(project_dir: &Path) -> String {
    claude_install(project_dir).expect("claude_install");
    let settings_path = project_dir.join(".claude").join("settings.json");
    let text = std::fs::read_to_string(settings_path).expect("read settings.json");
    let settings: Value = serde_json::from_str(&text).expect("parse settings.json");
    let hooks = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("PreToolUse array");
    let read_hook = hooks
        .iter()
        .find(|h| h.get("matcher").and_then(Value::as_str) == Some("Read|Glob"))
        .expect("Read|Glob hook present");
    read_hook["hooks"][0]["command"]
        .as_str()
        .expect("command string")
        .to_string()
}

/// Run `sh -c <cmd>` with `stdin`, in `cwd`; optionally create the graph first.
fn run(cmd: &str, tool_input: &Value, cwd: &Path, graph: bool) -> std::process::Output {
    if graph {
        let out = cwd.join("graphify-out");
        std::fs::create_dir_all(&out).expect("mkdir graphify-out");
        std::fs::write(out.join("graph.json"), "{}").expect("write graph.json");
    }
    let stdin = json!({ "tool_input": tool_input }).to_string();
    run_raw(cmd, &stdin, cwd)
}

fn run_raw(cmd: &str, stdin: &str, cwd: &Path) -> std::process::Output {
    use std::io::Write as _;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sh");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait sh")
}

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn matcher_targets_read_and_glob() {
    let tmp = tempfile::tempdir().expect("tempdir");
    claude_install(tmp.path()).expect("install");
    let text =
        std::fs::read_to_string(tmp.path().join(".claude").join("settings.json")).expect("read");
    let settings: Value = serde_json::from_str(&text).expect("parse");
    let hooks = settings["hooks"]["PreToolUse"].as_array().expect("array");
    assert!(
        hooks
            .iter()
            .any(|h| h.get("matcher").and_then(Value::as_str) == Some("Read|Glob"))
    );
}

#[test]
fn silent_without_graph() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    let out = run(&cmd, &json!({"file_path": "src/app.py"}), tmp.path(), false);
    assert_eq!(stdout_of(&out).trim(), "");
}

#[test]
fn nudges_on_source_read_with_graph() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    let out = run(&cmd, &json!({"file_path": "src/app.py"}), tmp.path(), true);
    assert!(stdout_of(&out).contains("graphify query"));
}

#[test]
fn nudge_payload_is_valid_pretooluse_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    let out = run(&cmd, &json!({"file_path": "pkg/mod.ts"}), tmp.path(), true);
    let payload: Value = serde_json::from_str(stdout_of(&out).trim()).expect("valid json");
    assert_eq!(
        payload["hookSpecificOutput"]["hookEventName"],
        json!("PreToolUse")
    );
    assert!(
        payload["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext")
            .contains("graphify query")
    );
}

#[test]
fn silent_on_graphify_out_targets() {
    // Reading the graph's own report must not start a go-read-the-graph loop.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    let out = run(
        &cmd,
        &json!({"file_path": "graphify-out/GRAPH_REPORT.md"}),
        tmp.path(),
        true,
    );
    assert_eq!(stdout_of(&out).trim(), "");
}

#[test]
fn silent_on_non_source_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    for path in ["uv.lock", "logo.png", "data.bin", ".gitignore"] {
        let out = run(&cmd, &json!({ "file_path": path }), tmp.path(), true);
        assert_eq!(stdout_of(&out).trim(), "", "{path} should not nudge");
    }
}

#[test]
fn glob_pattern_nudges() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    let out = run(
        &cmd,
        &json!({"pattern": "**/*.py", "path": "src"}),
        tmp.path(),
        true,
    );
    assert!(stdout_of(&out).contains("graphify query"));
}

#[test]
fn fails_open_on_malformed_stdin() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    let out_dir = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&out_dir).expect("mkdir");
    std::fs::write(out_dir.join("graph.json"), "{}").expect("write");
    let out = run_raw(&cmd, "this is not json", tmp.path());
    assert!(out.status.success());
    assert_eq!(stdout_of(&out).trim(), "");
}

#[test]
fn never_blocks() {
    // A nudge is additionalContext only — the hook must exit 0, never deny.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cmd = read_hook_command(tmp.path());
    let out = run(&cmd, &json!({"file_path": "src/app.py"}), tmp.path(), true);
    assert!(out.status.success());
    let s = stdout_of(&out);
    assert!(!s.contains("\"permissionDecision\""));
    assert!(!s.contains("\"deny\""));
}
