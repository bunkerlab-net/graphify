//! Install/uninstall graphify for VS Code Copilot Chat.
//!
//! Manages `.github/copilot-instructions.md` and the skill file at
//! `~/.copilot/skills/graphify/SKILL.md`. Kept separate because VS Code uses
//! its own instructions file path and skill destination distinct from the CLI copilot target.

use std::fs;
use std::path::Path;

use super::common::{
    CLAUDE_MD_MARKER, SKILL_VSCODE_MD, VSCODE_INSTRUCTIONS_SECTION, dirs_home, install_skill,
    remove_graphify_section, remove_skill, replace_or_append_section,
};
use crate::HooksError;

/// Install graphify skill + `.github/copilot-instructions.md` section for VS Code Copilot Chat.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn vscode_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_VSCODE_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    let instructions = project_dir.join(".github").join("copilot-instructions.md");
    if let Some(parent) = instructions.parent() {
        fs::create_dir_all(parent)?;
    }
    let (new_content, label) = if instructions.exists() {
        let content = fs::read_to_string(&instructions)?;
        let new =
            replace_or_append_section(&content, CLAUDE_MD_MARKER, VSCODE_INSTRUCTIONS_SECTION);
        let label = if new == content {
            "already configured (no change)"
        } else if content.contains(CLAUDE_MD_MARKER) {
            "updated"
        } else {
            "added"
        };
        (new, label)
    } else {
        (VSCODE_INSTRUCTIONS_SECTION.to_string(), "created")
    };

    if instructions.exists() && label == "already configured (no change)" {
        msgs.push(format!("  {}  ->  {label}", instructions.display()));
    } else {
        fs::write(&instructions, new_content.as_bytes())?;
        msgs.push(format!("  {}  ->  {label}", instructions.display()));
    }

    msgs.push(String::new());
    msgs.push(
        "VS Code Copilot Chat configured. Type /graphify in the chat panel to build the graph."
            .to_string(),
    );
    msgs.push("Note: for GitHub Copilot CLI (terminal), use: graphify copilot install".to_string());
    Ok(msgs.join("\n"))
}

/// Remove graphify VS Code Copilot Chat skill and `.github/copilot-instructions.md` section.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn vscode_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        msgs.push(format!("  skill removed    ->  {}", skill_dst.display()));
    }
    remove_skill(&skill_dst);

    let instructions = project_dir.join(".github").join("copilot-instructions.md");
    if !instructions.exists() {
        return Ok(msgs.join("\n"));
    }
    let content = fs::read_to_string(&instructions)?;
    if !content.contains(CLAUDE_MD_MARKER) {
        return Ok(msgs.join("\n"));
    }
    let cleaned = remove_graphify_section(&content);
    if cleaned.is_empty() {
        fs::remove_file(&instructions)?;
        msgs.push(format!(
            "  {}  ->  deleted (was empty after removal)",
            instructions.display()
        ));
    } else {
        fs::write(&instructions, format!("{cleaned}\n").as_bytes())?;
        msgs.push(format!(
            "  graphify section removed from {}",
            instructions.display()
        ));
    }
    Ok(msgs.join("\n"))
}
