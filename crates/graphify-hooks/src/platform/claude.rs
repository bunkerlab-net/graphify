//! Install/uninstall graphify for Claude Code.
//!
//! Manages the `CLAUDE.md` project context section and the `PreToolUse` hook
//! in `.claude/settings.json`. Kept in its own file because the hook JSON
//! schema differs from every other platform.

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::common::{
    CLAUDE_MD_MARKER, CLAUDE_MD_SECTION, SETTINGS_HOOK_MATCHER, read_json_or_empty,
    remove_graphify_section, replace_or_append_section, settings_hook, write_json,
};
use crate::HooksError;

/// Write the graphify section to `project_dir/CLAUDE.md` and install the
/// `PreToolUse` hook in `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn claude_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("CLAUDE.md");

    let new_content = if target.exists() {
        let content = fs::read_to_string(&target)?;
        replace_or_append_section(&content, CLAUDE_MD_MARKER, CLAUDE_MD_SECTION)
    } else {
        CLAUDE_MD_SECTION.trim_start().to_string()
    };

    if target.exists() && fs::read_to_string(&target).is_ok_and(|c| c == new_content) {
        msgs.push(format!(
            "graphify already configured in {} (no change)",
            target.display()
        ));
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, new_content.as_bytes())?;
        msgs.push(format!("graphify section written to {}", target.display()));
    }

    msgs.push(install_claude_hook(project_dir)?);

    msgs.push(String::new());
    msgs.push("Claude Code will now check the knowledge graph before answering".to_string());
    msgs.push("codebase questions and rebuild it after code changes.".to_string());

    Ok(msgs.join("\n"))
}

/// Remove the graphify section from `project_dir/CLAUDE.md` and remove the
/// `PreToolUse` hook from `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn claude_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("CLAUDE.md");

    if target.exists() {
        let content = fs::read_to_string(&target)?;
        if content.contains(CLAUDE_MD_MARKER) {
            let cleaned = remove_graphify_section(&content);
            if cleaned.is_empty() {
                fs::remove_file(&target)?;
                msgs.push(format!(
                    "CLAUDE.md was empty after removal - deleted {}",
                    target.display()
                ));
            } else {
                fs::write(&target, format!("{cleaned}\n").as_bytes())?;
                msgs.push(format!(
                    "graphify section removed from {}",
                    target.display()
                ));
            }
        } else {
            msgs.push("graphify section not found in CLAUDE.md - nothing to do".to_string());
        }
    } else {
        msgs.push("No CLAUDE.md found in current directory - nothing to do".to_string());
    }

    msgs.push(uninstall_claude_hook(project_dir)?);
    Ok(msgs.join("\n"))
}

/// Add graphify `PreToolUse` hook to `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_claude_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".claude").join("settings.json");
    let mut settings = read_json_or_empty(&settings_path);

    let hooks = settings
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
        })
        .ok_or_else(|| HooksError::Json("hooks is not an object".to_string()))?;

    let pre_tool = hooks
        .entry("PreToolUse")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(arr) = pre_tool {
        arr.retain(|h| {
            let matcher = h.get("matcher").and_then(Value::as_str).unwrap_or("");
            let is_stale_matcher = matcher == "Glob|Grep" || matcher == SETTINGS_HOOK_MATCHER;
            let has_graphify = h.to_string().contains("graphify");
            !(is_stale_matcher && has_graphify)
        });
        arr.push(settings_hook());
    }

    write_json(&settings_path, &settings)?;
    Ok("  .claude/settings.json  ->  PreToolUse hook registered".to_string())
}

/// Remove graphify `PreToolUse` hook from `project_dir/.claude/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_claude_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".claude").join("settings.json");
    if !settings_path.exists() {
        return Ok(String::new());
    }
    let mut settings = read_json_or_empty(&settings_path);
    let pre_tool = settings
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut);
    let Some(arr) = pre_tool else {
        return Ok(String::new());
    };
    let before = arr.len();
    arr.retain(|h| {
        let matcher = h.get("matcher").and_then(Value::as_str).unwrap_or("");
        let is_stale = matcher == "Glob|Grep" || matcher == SETTINGS_HOOK_MATCHER;
        !(is_stale && h.to_string().contains("graphify"))
    });
    if arr.len() == before {
        return Ok(String::new());
    }
    write_json(&settings_path, &settings)?;
    Ok("  .claude/settings.json  ->  PreToolUse hook removed".to_string())
}
