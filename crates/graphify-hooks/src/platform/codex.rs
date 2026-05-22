//! Install/uninstall graphify for `OpenAI` Codex.
//!
//! Codex uses `.codex/hooks.json` for `PreToolUse` integration — a separate
//! JSON structure from Claude's `.claude/settings.json`. This module handles
//! only the hook file; the `AGENTS.md` context file is managed by `agents.rs`.

use std::path::Path;

use serde_json::Value;

use super::common::{read_json_or_empty, resolve_graphify_exe, write_json};
use crate::HooksError;

/// Add graphify `PreToolUse` hook to `project_dir/.codex/hooks.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_codex_hook(project_dir: &Path) -> Result<String, HooksError> {
    let hooks_path = project_dir.join(".codex").join("hooks.json");
    if let Some(parent) = hooks_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut existing = read_json_or_empty(&hooks_path);

    let graphify_exe = resolve_graphify_exe();
    let hook_entry = serde_json::json!({
        "matcher": "Bash",
        "hooks": [{"type": "command", "command": format!("{graphify_exe} hook-check")}]
    });

    let pre_tool = existing
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
        })
        .map(|h| {
            h.entry("PreToolUse")
                .or_insert_with(|| Value::Array(Vec::new()))
        })
        .ok_or_else(|| HooksError::Json("PreToolUse is not valid".to_string()))?;

    if let Value::Array(arr) = pre_tool {
        arr.retain(|h| !h.to_string().contains("graphify"));
        arr.push(hook_entry);
    }

    write_json(&hooks_path, &existing)?;
    Ok(format!(
        "  .codex/hooks.json  ->  PreToolUse hook registered ({graphify_exe} hook-check)"
    ))
}

/// Remove graphify `PreToolUse` hook from `project_dir/.codex/hooks.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_codex_hook(project_dir: &Path) -> Result<String, HooksError> {
    let hooks_path = project_dir.join(".codex").join("hooks.json");
    if !hooks_path.exists() {
        return Ok(String::new());
    }
    let mut existing = read_json_or_empty(&hooks_path);
    let pre_tool = existing
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut);
    let Some(arr) = pre_tool else {
        return Ok(String::new());
    };
    let before = arr.len();
    arr.retain(|h| !h.to_string().contains("graphify"));
    if arr.len() == before {
        return Ok(String::new());
    }
    write_json(&hooks_path, &existing)?;
    Ok("  .codex/hooks.json  ->  PreToolUse hook removed".to_string())
}
