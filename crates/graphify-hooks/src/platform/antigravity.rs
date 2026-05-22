//! Install/uninstall graphify for Google Antigravity.
//!
//! Antigravity requires three artefacts: a home-dir skill at
//! `~/.agents/skills/graphify/SKILL.md`, a project-local rules file at
//! `.agents/rules/graphify.md`, and a workflow file at
//! `.agents/workflows/graphify.md`. The skill also needs a YAML frontmatter
//! block injected post-install, which is unique to this platform.

use std::fs;
use std::path::Path;

use super::common::{
    ANTIGRAVITY_RULES, ANTIGRAVITY_WORKFLOW, SKILL_MD, dirs_home, install_skill, remove_skill,
};
use crate::HooksError;

const ANTIGRAVITY_RULES_PATH: &str = ".agents/rules/graphify.md";
const ANTIGRAVITY_WORKFLOW_PATH: &str = ".agents/workflows/graphify.md";

/// Install graphify for Google Antigravity: skill + rules + workflows.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn antigravity_install(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = dirs_home()
        .join(".agents")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    install_skill(SKILL_MD, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));

    if skill_dst.exists() {
        let content = fs::read_to_string(&skill_dst)?;
        if !content.starts_with("---\n") {
            let frontmatter = "---\nname: graphify-manager\ndescription: Rebuild the code graph or perform manual CLI queries when MCP server is offline.\n---\n\n";
            fs::write(&skill_dst, format!("{frontmatter}{content}").as_bytes())?;
        }
    }

    let rules_path = project_dir.join(ANTIGRAVITY_RULES_PATH);
    if let Some(parent) = rules_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if rules_path.exists() {
        let existing = fs::read_to_string(&rules_path)?;
        if existing.trim() == ANTIGRAVITY_RULES.trim() {
            msgs.push(format!(
                "graphify rule already configured at {} (no change)",
                rules_path.display()
            ));
        } else {
            fs::write(&rules_path, ANTIGRAVITY_RULES.as_bytes())?;
            msgs.push(format!("graphify rule updated at {}", rules_path.display()));
        }
    } else {
        fs::write(&rules_path, ANTIGRAVITY_RULES.as_bytes())?;
        msgs.push(format!("graphify rule written to {}", rules_path.display()));
    }

    let wf_path = project_dir.join(ANTIGRAVITY_WORKFLOW_PATH);
    if let Some(parent) = wf_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if wf_path.exists() {
        let existing = fs::read_to_string(&wf_path)?;
        if existing.trim() == ANTIGRAVITY_WORKFLOW.trim() {
            msgs.push(format!(
                "graphify workflow already configured at {} (no change)",
                wf_path.display()
            ));
        } else {
            fs::write(&wf_path, ANTIGRAVITY_WORKFLOW.as_bytes())?;
            msgs.push(format!(
                "graphify workflow updated at {}",
                wf_path.display()
            ));
        }
    } else {
        fs::write(&wf_path, ANTIGRAVITY_WORKFLOW.as_bytes())?;
        msgs.push(format!(
            "graphify workflow written to {}",
            wf_path.display()
        ));
    }

    msgs.push(String::new());
    msgs.push("Antigravity will now check the knowledge graph before answering".to_string());
    msgs.push("codebase questions. Run /graphify first to build the graph.".to_string());
    msgs.push(String::new());
    msgs.push(
        "To enable full MCP architecture navigation, add this to ~/.gemini/antigravity/mcp_config.json:".to_string(),
    );
    msgs.push("  \"graphify\": {".to_string());
    msgs.push("    \"command\": \"uv\",".to_string());
    msgs.push("    \"args\": [\"run\", \"--with\", \"graphifyy\", \"--with\", \"mcp\", \"-m\", \"graphify.serve\", \"${workspace.path}/graphify-out/graph.json\"]".to_string());
    msgs.push("  }".to_string());

    Ok(msgs.join("\n"))
}

/// Remove graphify Antigravity rules, workflow, and skill files.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn antigravity_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let rules_path = project_dir.join(ANTIGRAVITY_RULES_PATH);
    if rules_path.exists() {
        fs::remove_file(&rules_path)?;
        msgs.push(format!(
            "graphify rule removed from {}",
            rules_path.display()
        ));
    } else {
        msgs.push("No graphify Antigravity rule found - nothing to do".to_string());
    }

    let wf_path = project_dir.join(ANTIGRAVITY_WORKFLOW_PATH);
    if wf_path.exists() {
        fs::remove_file(&wf_path)?;
        msgs.push(format!(
            "graphify workflow removed from {}",
            wf_path.display()
        ));
    }

    let skill_dst = dirs_home()
        .join(".agents")
        .join("skills")
        .join("graphify")
        .join("SKILL.md");
    if skill_dst.exists() {
        msgs.push(format!(
            "graphify skill removed from {}",
            skill_dst.display()
        ));
        remove_skill(&skill_dst);
    }

    Ok(msgs.join("\n"))
}
