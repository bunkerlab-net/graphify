//! Install/uninstall graphify for `CodeBuddy` (#1136).
//!
//! `CodeBuddy` reuses Claude Code's mechanism retargeted to `CODEBUDDY.md` and
//! `.codebuddy/settings.json`: the same `## graphify` section text
//! ([`CLAUDE_MD_SECTION`]) and the byte-identical `PreToolUse` hook pair (via
//! the shared [`register_pretooluse_hooks`] / [`remove_pretooluse_hooks`]
//! helpers). Unlike `claude_install`, `codebuddy_install` also drops the skill
//! file, mirroring graphify-py's `codebuddy_install`, which calls
//! `_copy_skill_file("codebuddy", ...)`.

use std::fs;
use std::path::{Path, PathBuf};

use super::common::{
    CLAUDE_MD_MARKER, CLAUDE_MD_SECTION, SKILL_MD, dirs_home, install_skill, read_json_or_empty,
    register_pretooluse_hooks, remove_graphify_section, remove_pretooluse_hooks, remove_skill,
    replace_or_append_section, write_json,
};
use crate::HooksError;

/// Project-scope skill destination for `CodeBuddy`
/// (`<project>/.codebuddy/skills/graphify/SKILL.md`).
#[must_use]
fn codebuddy_project_skill_dst(project_dir: &Path) -> PathBuf {
    project_dir
        .join(".codebuddy")
        .join("skills")
        .join("graphify")
        .join("SKILL.md")
}

/// User-scope skill destination for `CodeBuddy`
/// (`~/.codebuddy/skills/graphify/SKILL.md`).
///
/// `CodeBuddy` keys off `Path.home()` directly and has no `CLAUDE_CONFIG_DIR`
/// override, so this is always anchored at the home directory.
#[must_use]
fn codebuddy_user_skill_dst() -> PathBuf {
    dirs_home()
        .join(".codebuddy")
        .join("skills")
        .join("graphify")
        .join("SKILL.md")
}

/// Copy the graphify skill to `project_dir/.codebuddy/...`, write the graphify
/// section to `project_dir/CODEBUDDY.md`, and install the `PreToolUse` hook in
/// `project_dir/.codebuddy/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn codebuddy_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = codebuddy_project_skill_dst(project_dir);
    install_skill(SKILL_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    let target = project_dir.join("CODEBUDDY.md");
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

    msgs.push(install_codebuddy_hook(project_dir)?);

    msgs.push(String::new());
    msgs.push("CodeBuddy will now check the knowledge graph before answering".to_string());
    msgs.push("codebase questions and rebuild it after code changes.".to_string());

    Ok(msgs.join("\n"))
}

/// Remove the user-scope skill tree (`SKILL.md` + version stamp), the graphify
/// section from `project_dir/CODEBUDDY.md`, and the `PreToolUse` hook from
/// `project_dir/.codebuddy/settings.json`.
///
/// Mirrors `claude_uninstall`: the user-scope skill is removed first so a bare
/// `graphify codebuddy uninstall` / `graphify uninstall` does not orphan it,
/// matching graphify-py's `_remove_skill_file("codebuddy", project=False)`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn codebuddy_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = codebuddy_user_skill_dst();
    if skill_dst.exists() {
        msgs.push(format!("  skill removed    ->  {}", skill_dst.display()));
    }
    remove_skill(&skill_dst);

    let target = project_dir.join("CODEBUDDY.md");
    if target.exists() {
        let content = fs::read_to_string(&target)?;
        if content.contains(CLAUDE_MD_MARKER) {
            let cleaned = remove_graphify_section(&content);
            if cleaned.is_empty() {
                fs::remove_file(&target)?;
                msgs.push(format!(
                    "CODEBUDDY.md was empty after removal - deleted {}",
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
            msgs.push("graphify section not found in CODEBUDDY.md - nothing to do".to_string());
        }
    } else {
        msgs.push("No CODEBUDDY.md found in current directory - nothing to do".to_string());
    }

    msgs.push(uninstall_codebuddy_hook(project_dir)?);
    Ok(msgs.join("\n"))
}

/// Add the graphify `PreToolUse` hooks to `project_dir/.codebuddy/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_codebuddy_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".codebuddy").join("settings.json");
    let mut settings = read_json_or_empty(&settings_path);
    register_pretooluse_hooks(&mut settings)?;
    write_json(&settings_path, &settings)?;
    Ok("  .codebuddy/settings.json  ->  PreToolUse hooks registered".to_string())
}

/// Remove the graphify `PreToolUse` hooks from
/// `project_dir/.codebuddy/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_codebuddy_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".codebuddy").join("settings.json");
    if !settings_path.exists() {
        return Ok(String::new());
    }
    let mut settings = read_json_or_empty(&settings_path);
    if !remove_pretooluse_hooks(&mut settings) {
        return Ok(String::new());
    }
    write_json(&settings_path, &settings)?;
    Ok("  .codebuddy/settings.json  ->  PreToolUse hook removed".to_string())
}
