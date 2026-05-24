//! Install/uninstall graphify for Pi coding agent.
//!
//! Skill-file-only install targeting `~/.pi/agent/skills/graphify/SKILL.md`.
//! Pi has no project-level config file to modify, so install and uninstall
//! are minimal.

use super::common::{SKILL_PI_MD, dirs_home, install_skill, remove_skill};
use crate::HooksError;

/// Install graphify skill for Pi coding agent.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn pi_install() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".pi")
        .join("agent")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_PI_MD, &skill_dst)?;
    Ok(format!("  skill installed  ->  {}", skill_dst.display()))
}

/// Remove graphify skill for Pi coding agent.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn pi_uninstall() -> Result<String, HooksError> {
    let skill_dst = dirs_home()
        .join(".pi")
        .join("agent")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        let msg = format!("  skill removed    ->  {}", skill_dst.display());
        remove_skill(&skill_dst);
        Ok(msg)
    } else {
        Ok(String::new())
    }
}
