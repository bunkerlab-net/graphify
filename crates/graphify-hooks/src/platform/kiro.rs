//! Install/uninstall graphify for Kiro IDE/CLI.
//!
//! Kiro uses two project-local files: a skill at `.kiro/skills/graphify/SKILL.md`
//! and an always-on steering document at `.kiro/steering/graphify.md`. Both are
//! wholly owned by graphify and are overwritten on upgrade, which is why Kiro
//! warrants its own module.

use std::fs;
use std::path::Path;

use super::common::{KIRO_STEERING, SKILL_KIRO_MD, install_skill};
use crate::HooksError;

/// Install graphify skill + steering file for Kiro IDE/CLI.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn kiro_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = project_dir
        .join(".kiro")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_KIRO_MD, &skill_dst)?;
    msgs.push("  .kiro/skills/graphify/SKILL.md  ->  /graphify skill".to_string());

    let steering_dir = project_dir.join(".kiro").join("steering");
    fs::create_dir_all(&steering_dir)?;
    let steering_dst = steering_dir.join("graphify.md");
    let current = if steering_dst.exists() {
        fs::read_to_string(&steering_dst)?
    } else {
        String::new()
    };
    if current == KIRO_STEERING {
        msgs.push("  .kiro/steering/graphify.md  ->  already configured (no change)".to_string());
    } else {
        let action = if steering_dst.exists() {
            "updated"
        } else {
            "written"
        };
        fs::write(&steering_dst, KIRO_STEERING.as_bytes())?;
        msgs.push(format!(
            "  .kiro/steering/graphify.md  ->  always-on steering {action}"
        ));
    }

    msgs.push(String::new());
    msgs.push("Kiro will now read the knowledge graph before every conversation.".to_string());
    msgs.push("Use /graphify to build or update the graph.".to_string());
    Ok(msgs.join("\n"))
}

/// Remove graphify skill + steering file for Kiro.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn kiro_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut removed: Vec<String> = Vec::new();

    let skill_dst = project_dir
        .join(".kiro")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        fs::remove_file(&skill_dst)?;
        removed.push(".kiro/skills/graphify/SKILL.md".to_string());
        if let Some(p) = skill_dst.parent() {
            let _ = fs::remove_dir(p);
        }
    }

    let steering_dst = project_dir
        .join(".kiro")
        .join("steering")
        .join("graphify.md");
    if steering_dst.exists() {
        fs::remove_file(&steering_dst)?;
        removed.push(".kiro/steering/graphify.md".to_string());
    }

    if removed.is_empty() {
        Ok("Removed: nothing to remove".to_string())
    } else {
        Ok(format!("Removed: {}", removed.join(", ")))
    }
}
