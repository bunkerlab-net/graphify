//! Install/uninstall graphify for Claude Code.
//!
//! Manages the `CLAUDE.md` project context section and the `PreToolUse` hook
//! in `.claude/settings.json`. Kept in its own file because the hook JSON
//! schema differs from every other platform.

use std::fs;
use std::path::{Path, PathBuf};

use super::common::{
    CLAUDE_MD_MARKER, CLAUDE_MD_SECTION, claude_config_dir, dirs_home, read_json_or_empty,
    register_pretooluse_hooks, remove_graphify_section, remove_pretooluse_hooks, remove_skill,
    replace_or_append_section, write_json,
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

/// User-scope Claude skill destination (`SKILL.md`), honouring
/// `CLAUDE_CONFIG_DIR`. Mirrors the path used by `install_platform_skill`.
#[must_use]
fn claude_user_skill_dst() -> PathBuf {
    // `claude_config_dir()` treats an empty `CLAUDE_CONFIG_DIR` as unset so this
    // never collapses to a stray relative path the installer would never match.
    claude_config_dir()
        .unwrap_or_else(|| dirs_home().join(".claude"))
        .join("skills")
        .join("graphify")
        .join("SKILL.md")
}

/// Remove the graphify skill tree (`SKILL.md` + version stamp), the graphify
/// section from `project_dir/CLAUDE.md`, and the `PreToolUse` hook from
/// `project_dir/.claude/settings.json`.
///
/// Mirrors `gemini_uninstall`: a bare `graphify uninstall` / `graphify claude
/// uninstall` must also remove the installed skill, not just strip CLAUDE.md, or
/// the user-scope skill is orphaned (#1121). Project-scope skill removal is
/// handled separately by `uninstall_platform_skill_project`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn claude_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    // Remove the user-scope skill tree first, mirroring Python's
    // `_remove_skill_file("claude", project=False)` ordering. The message is
    // emitted when the skill is present (before the best-effort `remove_skill`,
    // which returns `()` and ignores errors) — the same ordering `gemini_uninstall`
    // uses, so a reviewer's "message before op" note is intentional here.
    let skill_dst = claude_user_skill_dst();
    if skill_dst.exists() {
        msgs.push(format!("  skill removed    ->  {}", skill_dst.display()));
    }
    remove_skill(&skill_dst);

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
    register_pretooluse_hooks(&mut settings)?;
    write_json(&settings_path, &settings)?;
    Ok(
        "  .claude/settings.json  ->  PreToolUse hooks registered (Bash search + Read/Glob)"
            .to_string(),
    )
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
    if !remove_pretooluse_hooks(&mut settings) {
        return Ok(String::new());
    }
    write_json(&settings_path, &settings)?;
    Ok("  .claude/settings.json  ->  PreToolUse hook removed".to_string())
}
