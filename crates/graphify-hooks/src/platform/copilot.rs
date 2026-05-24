//! Install/uninstall graphify for GitHub Copilot CLI.
//!
//! This is a skill-file-only install — no project configuration file is modified.
//! The skill lands at `~/.copilot/skills/graphify/SKILL.md`, which is the CLI
//! counterpart to the VS Code extension path managed by `vscode.rs`.

use super::common::{SKILL_COPILOT_MD, dirs_home, install_skill, remove_skill};
use crate::HooksError;

/// Install graphify skill for GitHub Copilot CLI (`~/.copilot/skills/graphify/SKILL.md`).
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn copilot_install() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_COPILOT_MD, &skill_dst)?;
    Ok(format!("  skill installed  ->  {}", skill_dst.display()))
}

/// Remove graphify skill for GitHub Copilot CLI.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn copilot_uninstall() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".copilot")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if !skill_dst.exists() {
        return Ok("nothing to remove".to_string());
    }
    let msg = format!("skill removed: {}", skill_dst.display());
    remove_skill(&skill_dst);
    Ok(msg)
}
