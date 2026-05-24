//! Install/uninstall graphify via `AGENTS.md` for multi-platform agents.
//!
//! This module covers codex, opencode, aider, claw, droid, trae, trae-cn, and hermes —
//! all of which read an `AGENTS.md` file for persistent context. Codex and `OpenCode` each
//! also register a platform-specific hook/plugin in addition to the shared markdown section.

use std::fs;
use std::path::Path;

use super::codex::{install_codex_hook, uninstall_codex_hook};
use super::common::{
    AGENTS_MD_SECTION, CLAUDE_MD_MARKER, remove_graphify_section, replace_or_append_section,
};
use super::opencode::{install_opencode_plugin, uninstall_opencode_plugin};
use crate::HooksError;

/// Write the graphify section to `project_dir/AGENTS.md`.
///
/// For `codex` also installs `.codex/hooks.json`.
/// For `opencode` also installs `.opencode/plugins/graphify.js`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn agents_install(project_dir: &Path, platform: &str) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("AGENTS.md");

    let new_content = if target.exists() {
        let content = fs::read_to_string(&target)?;
        replace_or_append_section(&content, CLAUDE_MD_MARKER, AGENTS_MD_SECTION)
    } else {
        AGENTS_MD_SECTION.to_string()
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

    if platform == "codex" {
        msgs.push(install_codex_hook(project_dir)?);
    } else if platform == "opencode" {
        msgs.push(install_opencode_plugin(project_dir)?);
    }

    let platform_cap = {
        let mut s = platform.to_string();
        if let Some(c) = s.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        s
    };

    msgs.push(String::new());
    msgs.push(format!(
        "{platform_cap} will now check the knowledge graph before answering"
    ));
    msgs.push("codebase questions and rebuild it after code changes.".to_string());

    if !matches!(platform, "codex" | "opencode") {
        msgs.push(String::new());
        msgs.push(
            "Note: unlike Claude Code, there is no PreToolUse hook equivalent for".to_string(),
        );
        msgs.push(format!(
            "{platform_cap} — the AGENTS.md rules are the always-on mechanism."
        ));
    }

    Ok(msgs.join("\n"))
}

/// Remove the graphify section from `project_dir/AGENTS.md`.
///
/// For `opencode` also removes the `OpenCode` plugin.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn agents_uninstall(project_dir: &Path, platform: &str) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let target = project_dir.join("AGENTS.md");

    if !target.exists() {
        msgs.push("No AGENTS.md found in current directory - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }

    let content = fs::read_to_string(&target)?;
    if !content.contains(CLAUDE_MD_MARKER) {
        msgs.push("graphify section not found in AGENTS.md - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }

    let cleaned = remove_graphify_section(&content);
    if cleaned.is_empty() {
        fs::remove_file(&target)?;
        msgs.push(format!(
            "AGENTS.md was empty after removal - deleted {}",
            target.display()
        ));
    } else {
        fs::write(&target, format!("{cleaned}\n").as_bytes())?;
        msgs.push(format!(
            "graphify section removed from {}",
            target.display()
        ));
    }

    if platform == "opencode" {
        msgs.push(uninstall_opencode_plugin(project_dir)?);
    } else if platform == "codex" {
        msgs.push(uninstall_codex_hook(project_dir)?);
    }

    Ok(msgs.join("\n"))
}
