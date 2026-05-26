//! Install/uninstall graphify for Devin CLI.
//!
//! Devin requires two artefacts: a skill file at
//! `~/.config/devin/skills/graphify/SKILL.md` (or
//! `<project>/.devin/skills/graphify/SKILL.md` under `--project`) plus a
//! `.windsurf/rules/graphify.md` always-on context file in the project
//! root — Devin reads `.windsurf/rules/*.md` the same way the Windsurf IDE
//! does.

use std::fs;
use std::path::Path;

use super::common::{SKILL_MD, dirs_home, install_skill, remove_skill};
use crate::HooksError;

const DEVIN_RULES_PATH: &str = ".windsurf/rules/graphify.md";

/// Always-on graphify context for Devin sessions, written to
/// `.windsurf/rules/graphify.md`.
pub(super) const DEVIN_RULES: &str = "## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- For codebase or architecture questions, when `graphify-out/graph.json` exists, first run `graphify query \"<question>\"` (or `graphify path \"<A>\" \"<B>\"` / `graphify explain \"<concept>\"`). These return a scoped subgraph, usually much smaller than `GRAPH_REPORT.md` or raw grep output.
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context
- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)
";

fn user_skill_path() -> std::path::PathBuf {
    dirs_home()
        .join(".config")
        .join("devin")
        .join("skills")
        .join("graphify")
        .join("SKILL.md")
}

fn project_skill_path(project_dir: &Path) -> std::path::PathBuf {
    project_dir
        .join(".devin")
        .join("skills")
        .join("graphify")
        .join("SKILL.md")
}

/// Install graphify skill for the Devin CLI at user scope.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn devin_install() -> Result<String, HooksError> {
    let skill_dst = user_skill_path();
    install_skill(SKILL_MD, &skill_dst)?;
    Ok(format!("  skill installed  ->  {}", skill_dst.display()))
}

/// Remove graphify skill for the Devin CLI at user scope.
///
/// Skill removal is best-effort: the shared `remove_skill` helper swallows
/// filesystem errors so callers see a clean "skill removed" message even
/// if a leftover empty directory could not be reaped. Matches the
/// behaviour of `pi_uninstall`.
///
/// # Errors
///
/// Currently never returns an error. The `Result` return type is kept for
/// symmetry with [`devin_install`] and [`devin_project_uninstall`], which
/// can fail when reading or writing `.windsurf/rules/graphify.md`.
pub fn devin_uninstall() -> Result<String, HooksError> {
    let skill_dst = user_skill_path();
    if skill_dst.exists() {
        let msg = format!("  skill removed    ->  {}", skill_dst.display());
        remove_skill(&skill_dst);
        Ok(msg)
    } else {
        Ok("nothing to remove".to_string())
    }
}

/// Install graphify for Devin under `project_dir`: the project-scoped skill
/// plus `.windsurf/rules/graphify.md`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn devin_project_install(project_dir: &Path) -> Result<String, HooksError> {
    let skill_dst = project_skill_path(project_dir);
    install_skill(SKILL_MD, &skill_dst)?;
    let mut msgs = vec![format!("  skill installed  ->  {}", skill_dst.display())];

    let rules_path = project_dir.join(DEVIN_RULES_PATH);
    if let Some(parent) = rules_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if rules_path.exists() && fs::read_to_string(&rules_path)? == DEVIN_RULES {
        msgs.push(format!(
            "  {}  ->  already configured (no change)",
            rules_path.display()
        ));
    } else {
        let action = if rules_path.exists() {
            "updated"
        } else {
            "written"
        };
        fs::write(&rules_path, DEVIN_RULES.as_bytes())?;
        msgs.push(format!("  rules {action}  ->  {}", rules_path.display()));
    }

    Ok(msgs.join("\n"))
}

/// Uninstall graphify for Devin under `project_dir`: removes the
/// project-scoped skill plus `.windsurf/rules/graphify.md`.
///
/// Skill removal is best-effort (see [`devin_uninstall`]). The rules file
/// removal does propagate I/O errors so a permissions failure on
/// `.windsurf/rules/graphify.md` is surfaced.
///
/// # Errors
///
/// Returns `HooksError::Io` if removing `.windsurf/rules/graphify.md`
/// fails. Failures while reaping the skill file itself are swallowed by
/// the shared `remove_skill` helper.
pub fn devin_project_uninstall(project_dir: &Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();

    let skill_dst = project_skill_path(project_dir);
    let removed_skill = skill_dst.exists();
    if removed_skill {
        remove_skill(&skill_dst);
        msgs.push(format!("  skill removed    ->  {}", skill_dst.display()));
    }

    let rules_path = project_dir.join(DEVIN_RULES_PATH);
    if rules_path.exists() {
        fs::remove_file(&rules_path)?;
        msgs.push(format!("  rules removed    ->  {}", rules_path.display()));
    }

    if msgs.is_empty() {
        Ok("nothing to remove".to_string())
    } else {
        Ok(msgs.join("\n"))
    }
}
