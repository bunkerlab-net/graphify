//! Install/uninstall graphify for Gemini CLI.
//!
//! Manages the `GEMINI.md` project context section, a `BeforeTool` hook in
//! `.gemini/settings.json`, and the skill file at `~/.gemini/skills/graphify/SKILL.md`.
//! Kept separate because Gemini has its own hook key (`BeforeTool`) and a platform-specific
//! skill destination that differs from every other agent.

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::common::{
    CLAUDE_MD_MARKER, GEMINI_MD_SECTION, SKILL_MD, dirs_home, gemini_hook, install_skill,
    read_json_or_empty, remove_graphify_section, remove_skill, replace_or_append_section,
    write_json,
};
use crate::HooksError;

/// Install graphify skill + GEMINI.md section + `BeforeTool` hook for Gemini CLI.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn gemini_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = if cfg!(target_os = "windows") {
        dirs_home()
            .join(".agents")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    } else {
        dirs_home()
            .join(".gemini")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    };
    install_skill(SKILL_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    let target = project_dir.join("GEMINI.md");
    let new_content = if target.exists() {
        let content = fs::read_to_string(&target)?;
        replace_or_append_section(&content, CLAUDE_MD_MARKER, GEMINI_MD_SECTION)
    } else {
        GEMINI_MD_SECTION.to_string()
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

    msgs.push(install_gemini_hook(project_dir)?);
    msgs.push(String::new());
    msgs.push("Gemini CLI will now check the knowledge graph before answering".to_string());
    msgs.push("codebase questions and rebuild it after code changes.".to_string());

    Ok(msgs.join("\n"))
}

/// Remove the graphify section from GEMINI.md, uninstall hook, and remove skill.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn gemini_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = if cfg!(target_os = "windows") {
        dirs_home()
            .join(".agents")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    } else {
        dirs_home()
            .join(".gemini")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    };
    if skill_dst.exists() {
        msgs.push(format!("  skill removed    ->  {}", skill_dst.display()));
    }
    remove_skill(&skill_dst);

    let target = project_dir.join("GEMINI.md");
    if !target.exists() {
        msgs.push("No GEMINI.md found in current directory - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }
    let content = fs::read_to_string(&target)?;
    if !content.contains(CLAUDE_MD_MARKER) {
        msgs.push("graphify section not found in GEMINI.md - nothing to do".to_string());
        return Ok(msgs.join("\n"));
    }
    let cleaned = remove_graphify_section(&content);
    if cleaned.is_empty() {
        fs::remove_file(&target)?;
        msgs.push(format!(
            "GEMINI.md was empty after removal - deleted {}",
            target.display()
        ));
    } else {
        fs::write(&target, format!("{cleaned}\n").as_bytes())?;
        msgs.push(format!(
            "graphify section removed from {}",
            target.display()
        ));
    }
    msgs.push(uninstall_gemini_hook(project_dir)?);
    Ok(msgs.join("\n"))
}

/// Add graphify `BeforeTool` hook to `project_dir/.gemini/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures or `HooksError::Json` on
/// JSON serialisation failure.
pub fn install_gemini_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".gemini").join("settings.json");
    let mut settings = read_json_or_empty(&settings_path);

    let hooks = settings
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
        })
        .ok_or_else(|| HooksError::Json("hooks is not an object".to_string()))?;

    let before_tool = hooks
        .entry("BeforeTool")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(arr) = before_tool {
        // Inspect the structured command strings rather than serialising the
        // whole entry; that avoids brittle whole-object substring matches.
        arr.retain(|h| {
            let has_graphify = h.get("hooks").and_then(Value::as_array).map_or_else(
                || {
                    h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|c| c.contains("graphify"))
                },
                |inner| {
                    inner.iter().any(|step| {
                        step.get("command")
                            .and_then(Value::as_str)
                            .is_some_and(|c| c.contains("graphify"))
                    })
                },
            );
            !has_graphify
        });
        arr.push(gemini_hook());
    }

    write_json(&settings_path, &settings)?;
    Ok("  .gemini/settings.json  ->  BeforeTool hook registered".to_string())
}

/// Remove graphify `BeforeTool` hook from `project_dir/.gemini/settings.json`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn uninstall_gemini_hook(project_dir: &Path) -> Result<String, HooksError> {
    let settings_path = project_dir.join(".gemini").join("settings.json");
    if !settings_path.exists() {
        return Ok(String::new());
    }
    let mut settings = read_json_or_empty(&settings_path);
    let before_tool = settings
        .pointer_mut("/hooks/BeforeTool")
        .and_then(Value::as_array_mut);
    let Some(arr) = before_tool else {
        return Ok(String::new());
    };
    let before = arr.len();
    arr.retain(|h| !h.to_string().contains("graphify"));
    if arr.len() == before {
        return Ok(String::new());
    }
    write_json(&settings_path, &settings)?;
    Ok("  .gemini/settings.json  ->  BeforeTool hook removed".to_string())
}
