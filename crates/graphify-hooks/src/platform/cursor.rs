//! Install/uninstall graphify for Cursor IDE.
//!
//! Manages `.cursor/rules/graphify.mdc` — a wholly-owned rule file with
//! `alwaysApply: true`. Because Cursor uses its own MDC format and directory
//! structure, install and uninstall are kept separate from other platforms.

use std::fs;
use std::path::Path;

use super::common::CURSOR_RULE;
use crate::HooksError;

/// Write `.cursor/rules/graphify.mdc` with `alwaysApply: true`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn cursor_install(project_dir: &Path) -> Result<String, HooksError> {
    let rule_path = project_dir
        .join(".cursor")
        .join("rules")
        .join("graphify.mdc");
    if let Some(parent) = rule_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if rule_path.exists() && fs::read_to_string(&rule_path).is_ok_and(|c| c == CURSOR_RULE) {
        return Ok(format!(
            "graphify rule at {} already configured (no change)",
            rule_path.display()
        ));
    }
    let action = if rule_path.exists() {
        "updated"
    } else {
        "written"
    };
    fs::write(&rule_path, CURSOR_RULE.as_bytes())?;
    let mut msgs = vec![format!("graphify rule {action} at {}", rule_path.display())];
    msgs.push(String::new());
    msgs.push("Cursor will now always include the knowledge graph context.".to_string());
    msgs.push("Run /graphify . first to build the graph if you haven't already.".to_string());
    Ok(msgs.join("\n"))
}

/// Remove `.cursor/rules/graphify.mdc`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn cursor_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let rule_path = project_dir
        .join(".cursor")
        .join("rules")
        .join("graphify.mdc");
    if !rule_path.exists() {
        return Ok("No graphify Cursor rule found - nothing to do".to_string());
    }
    fs::remove_file(&rule_path)?;
    Ok(format!(
        "graphify Cursor rule removed from {}",
        rule_path.display()
    ))
}
