//! Multi-platform skill installer used by every platform that only needs
//! a home-directory skill file (no project-local files).

use std::fs;

use crate::HooksError;

use super::fs::{claude_config_dir, dirs_home, install_skill};
use super::skills::{
    CODEBUDDY_REGISTRATION, SKILL_AIDER_MD, SKILL_CLAW_MD, SKILL_CODEX_MD, SKILL_COPILOT_MD,
    SKILL_DROID_MD, SKILL_KIRO_MD, SKILL_MD, SKILL_OPENCODE_MD, SKILL_PI_MD, SKILL_REGISTRATION,
    SKILL_TRAE_MD, SKILL_WINDOWS_MD,
};

/// Install a skill-only platform integration.
///
/// Maps `platform` to the correct skill content + destination path and
/// copies the file, mirroring the Python `install(platform=...)` function.
///
/// Supported: `claude`, `windows`, `codex`, `opencode`, `aider`, `copilot`,
/// `claw`, `droid`, `trae`, `trae-cn`, `hermes`, `kiro`, `pi`, `antigravity`,
/// `antigravity-windows`.
///
/// Also writes `~/.claude/CLAUDE.md` for `claude` and `windows` platforms,
/// honoring the `CLAUDE_CONFIG_DIR` environment variable if set.
///
/// # Errors
///
/// Returns `HooksError::UnknownPlatform` for unrecognised names,
/// `HooksError::Io` on filesystem failures.
pub fn install_platform_skill(platform: &str) -> Result<String, HooksError> {
    let (skill_content, home_rel) = skill_for(platform, false)?;

    let skill_dst = if matches!(platform, "claude" | "windows") {
        // Empty `CLAUDE_CONFIG_DIR` is treated as unset (see `claude_config_dir`)
        // so install and uninstall agree on the destination.
        match claude_config_dir() {
            Some(cfg_dir) => cfg_dir.join("skills").join("graphify").join("SKILL.md"),
            None => dirs_home().join(home_rel),
        }
    } else {
        dirs_home().join(home_rel)
    };

    install_skill(skill_content, &skill_dst)?;
    let mut msgs = vec![format!("  skill installed  ->  {}", skill_dst.display())];

    if matches!(platform, "claude" | "windows") {
        let claude_md = match claude_config_dir() {
            Some(cfg_dir) => cfg_dir.join("CLAUDE.md"),
            None => dirs_home().join(".claude").join("CLAUDE.md"),
        };
        if claude_md.exists() {
            let content = fs::read_to_string(&claude_md)?;
            if content.contains("graphify") {
                msgs.push("  CLAUDE.md        ->  already registered (no change)".to_string());
            } else {
                let new = format!("{}{}", content.trim_end(), SKILL_REGISTRATION);
                if let Some(parent) = claude_md.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&claude_md, new.as_bytes())?;
                msgs.push(format!(
                    "  CLAUDE.md        ->  skill registered in {}",
                    claude_md.display()
                ));
            }
        } else {
            if let Some(parent) = claude_md.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&claude_md, SKILL_REGISTRATION.trim_start().as_bytes())?;
            msgs.push(format!(
                "  CLAUDE.md        ->  created at {}",
                claude_md.display()
            ));
        }
    }

    // `CodeBuddy` registers the skill in its own user-scope context file
    // (`~/.codebuddy/CODEBUDDY.md`), mirroring graphify-py's `install("codebuddy")`.
    if platform == "codebuddy" {
        let codebuddy_md = dirs_home().join(".codebuddy").join("CODEBUDDY.md");
        if codebuddy_md.exists() {
            let content = fs::read_to_string(&codebuddy_md)?;
            if content.contains("graphify") {
                msgs.push("  CODEBUDDY.md     ->  already registered (no change)".to_string());
            } else {
                let new = format!("{}{}", content.trim_end(), CODEBUDDY_REGISTRATION);
                if let Some(parent) = codebuddy_md.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&codebuddy_md, new.as_bytes())?;
                msgs.push(format!(
                    "  CODEBUDDY.md     ->  skill registered in {}",
                    codebuddy_md.display()
                ));
            }
        } else {
            if let Some(parent) = codebuddy_md.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(
                &codebuddy_md,
                CODEBUDDY_REGISTRATION.trim_start().as_bytes(),
            )?;
            msgs.push(format!(
                "  CODEBUDDY.md     ->  created at {}",
                codebuddy_md.display()
            ));
        }
    }

    msgs.push(String::new());
    msgs.push("Done. Open your AI coding assistant and type:".to_string());
    msgs.push(String::new());
    msgs.push("  /graphify .".to_string());
    msgs.push(String::new());
    Ok(msgs.join("\n"))
}

/// Map a platform name to its skill content + relative install path.
///
/// `project` selects the project-scope path where it differs from the
/// user-scope path. `opencode` is the only platform with distinct paths:
/// `~/.config/opencode/...` at user scope but `./.opencode/...` at project
/// scope (mirrors graphify-py `_platform_skill_destination`).
fn skill_for(platform: &str, project: bool) -> Result<(&'static str, &'static str), HooksError> {
    Ok(match platform {
        "claude" | "windows" => {
            let skill = if platform == "windows" {
                SKILL_WINDOWS_MD
            } else {
                SKILL_MD
            };
            (skill, ".claude/skills/graphify/SKILL.md")
        }
        // Codex installs to `.codex/skills/...` (#1160): the hook already wrote
        // to `.codex/`, so the skill destination was previously inconsistent.
        "codex" => (SKILL_CODEX_MD, ".codex/skills/graphify/SKILL.md"),
        // `CodeBuddy` rides claude's skill bundle (#1136). The user-scope
        // CODEBUDDY.md registration is written by `install_platform_skill`.
        "codebuddy" => (SKILL_MD, ".codebuddy/skills/graphify/SKILL.md"),
        "amp" if project => (SKILL_MD, ".agents/skills/graphify/SKILL.md"),
        "amp" => (SKILL_MD, ".config/agents/skills/graphify/SKILL.md"),
        "opencode" if project => (SKILL_OPENCODE_MD, ".opencode/skills/graphify/SKILL.md"),
        "opencode" => (
            SKILL_OPENCODE_MD,
            ".config/opencode/skills/graphify/SKILL.md",
        ),
        "aider" => (SKILL_AIDER_MD, ".aider/graphify/SKILL.md"),
        "copilot" => (SKILL_COPILOT_MD, ".copilot/skills/graphify/SKILL.md"),
        "claw" => (SKILL_CLAW_MD, ".openclaw/skills/graphify/SKILL.md"),
        "droid" => (SKILL_DROID_MD, ".factory/skills/graphify/SKILL.md"),
        "trae" => (SKILL_TRAE_MD, ".trae/skills/graphify/SKILL.md"),
        "trae-cn" => (SKILL_TRAE_MD, ".trae-cn/skills/graphify/SKILL.md"),
        "hermes" => (SKILL_CLAW_MD, ".hermes/skills/graphify/SKILL.md"),
        "kiro" => (SKILL_KIRO_MD, ".kiro/skills/graphify/SKILL.md"),
        "pi" => (SKILL_PI_MD, ".pi/agent/skills/graphify/SKILL.md"),
        "kimi" => (SKILL_MD, ".kimi/skills/graphify/SKILL.md"),
        // Devin user-scope skill lives under `~/.config/devin/...`, but the
        // home-relative prefix differs from `.devin/...` used at project
        // scope. The project-scope install (devin install --project) is
        // handled by `devin_project_install`, which also writes
        // `.windsurf/rules/graphify.md`.
        "devin" => (SKILL_MD, ".config/devin/skills/graphify/SKILL.md"),
        "antigravity" => (SKILL_MD, ".agents/skills/graphify/SKILL.md"),
        "antigravity-windows" => (SKILL_WINDOWS_MD, ".agents/skills/graphify/SKILL.md"),
        other => return Err(HooksError::UnknownPlatform(other.to_string())),
    })
}

/// Project-scoped install: write the platform skill under `project_dir`
/// instead of `~/`, register it in the project's `CLAUDE.md` using a
/// relative path, and print a `git add` hint pointing at the newly
/// created files.
///
/// Mirrors the `--project` flag added in graphify-py
/// `__main__.py:_project_install`.
///
/// # Errors
///
/// Returns `HooksError::UnknownPlatform` for unrecognised names,
/// `HooksError::Io` on filesystem failures.
pub fn install_platform_skill_project(
    platform: &str,
    project_dir: &std::path::Path,
) -> Result<String, HooksError> {
    let (skill_content, rel) = skill_for(platform, true)?;
    let skill_dst = project_dir.join(rel);
    install_skill(skill_content, &skill_dst)?;
    let mut msgs = vec![format!("  skill installed  ->  {}", skill_dst.display())];

    if matches!(platform, "claude" | "windows") {
        let claude_md = project_dir.join(".claude").join("CLAUDE.md");
        let registration =
            format!("\n\n## graphify\n\nFollow `{rel}` when working in this project.\n");
        if claude_md.exists() {
            let content = fs::read_to_string(&claude_md)?;
            // Look for the exact registration heading on its own line, not
            // just any prefix match — a docs heading like
            // `## graphify internals` is not a registration entry and must
            // not block install.
            if content
                .lines()
                .any(|line| line.trim_start() == "## graphify")
            {
                msgs.push("  .claude/CLAUDE.md->  already registered (no change)".to_string());
            } else {
                let new = format!("{}{registration}", content.trim_end());
                fs::write(&claude_md, new.as_bytes())?;
                msgs.push(format!(
                    "  .claude/CLAUDE.md->  skill registered in {}",
                    claude_md.display()
                ));
            }
        } else {
            if let Some(parent) = claude_md.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&claude_md, registration.trim_start().as_bytes())?;
            msgs.push(format!(
                "  .claude/CLAUDE.md->  created at {}",
                claude_md.display()
            ));
        }
    }

    // Agents-group platforms also get the always-on AGENTS.md section (and the
    // codex hooks.json / opencode plugin) under the project, mirroring
    // graphify-py `_project_install`, which calls `_agents_install` for these.
    if is_agents_md_platform(platform) {
        msgs.push(crate::platform::agents::agents_install(
            project_dir,
            platform,
        )?);
    }

    msgs.push(String::new());
    let scope_root = scope_root_for(rel);
    msgs.push(format!("Don't forget to: git add {scope_root}"));
    msgs.push(String::new());
    Ok(msgs.join("\n"))
}

/// Project-scoped uninstall: remove only the project-local skill files
/// (and the registration in `.claude/CLAUDE.md`), leaving the user-global
/// install untouched.
///
/// # Errors
///
/// Returns `HooksError::UnknownPlatform` for unrecognised names,
/// `HooksError::Io` on filesystem failures.
pub fn uninstall_platform_skill_project(
    platform: &str,
    project_dir: &std::path::Path,
) -> Result<String, HooksError> {
    let (_skill_content, rel) = skill_for(platform, true)?;
    let skill_dst = project_dir.join(rel);
    let mut msgs: Vec<String> = Vec::new();
    if skill_dst.exists() {
        fs::remove_file(&skill_dst)?;
        msgs.push(format!("  removed  {}", skill_dst.display()));
        // Best-effort: prune empty ancestor directories on the way up to
        // (and including) the platform scope root (e.g. `.claude/`). The
        // walk stops at either the scope root or the project root itself,
        // and any error before that boundary (non-empty dir, permission
        // failure) breaks out of the loop. Errors are intentionally
        // ignored — leaving a non-empty dir behind is the correct
        // outcome, not a failure mode to surface.
        let scope_root = project_dir.join(scope_root_for(rel));
        let mut p = skill_dst.parent().map(std::path::Path::to_path_buf);
        while let Some(dir) = p {
            if dir == scope_root || dir == project_dir {
                // Try one last best-effort removal at the scope boundary,
                // then stop regardless of outcome.
                let _ = fs::remove_dir(&dir);
                break;
            }
            if fs::remove_dir(&dir).is_err() {
                break;
            }
            p = dir.parent().map(std::path::Path::to_path_buf);
        }
    } else {
        msgs.push(format!("  not installed at {}", skill_dst.display()));
    }
    if matches!(platform, "claude" | "windows") {
        let claude_md = project_dir.join(".claude").join("CLAUDE.md");
        if claude_md.exists() {
            // Propagate every I/O failure rather than swallowing it. The
            // documented contract of `uninstall_platform_skill_project`
            // promises `HooksError::Io` on filesystem failures, and a
            // silent swallow here would let the function report success
            // even when the CLAUDE.md update failed.
            let content = fs::read_to_string(&claude_md)?;
            let cleaned = strip_graphify_section(&content);
            if cleaned.trim().is_empty() {
                fs::remove_file(&claude_md)?;
            } else {
                fs::write(&claude_md, cleaned)?;
            }
        }
    }

    // Mirror the project-install agents wiring: drop the AGENTS.md section (and
    // the codex/opencode artefacts) for agents-group platforms.
    if is_agents_md_platform(platform) {
        msgs.push(crate::platform::agents::agents_uninstall(
            project_dir,
            platform,
        )?);
    }
    Ok(msgs.join("\n"))
}

/// True for platforms whose project install also writes an always-on
/// `AGENTS.md` section via [`crate::platform::agents::agents_install`]
/// (mirrors graphify-py `_project_install`'s `aider`/`amp`/`codex`/… branch).
fn is_agents_md_platform(platform: &str) -> bool {
    matches!(
        platform,
        "aider" | "amp" | "codex" | "opencode" | "claw" | "droid" | "trae" | "trae-cn" | "hermes"
    )
}

/// User-scope Amp install (mirrors graphify-py `_amp_install`).
///
/// Cleans the legacy `~/.amp/skills/graphify` directory older versions wrote,
/// installs the skill into `~/.config/agents/skills` (an Amp search root,
/// unlike the never-searched `~/.amp/skills`), and writes the always-on
/// `AGENTS.md` section into `project_dir`.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn amp_install(project_dir: &std::path::Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let legacy = dirs_home().join(".amp").join("skills").join("graphify");
    if legacy.exists() && fs::remove_dir_all(&legacy).is_ok() {
        msgs.push(format!("  legacy removed   ->  {}", legacy.display()));
    }
    let (skill_content, rel) = skill_for("amp", false)?;
    let skill_dst = dirs_home().join(rel);
    install_skill(skill_content, &skill_dst)?;
    msgs.push(format!("  skill installed  ->  {}", skill_dst.display()));
    msgs.push(crate::platform::agents::agents_install(project_dir, "amp")?);
    Ok(msgs.join("\n"))
}

/// User-scope Amp uninstall (mirrors graphify-py `_amp_uninstall`): removes the
/// `~/.config/agents/skills` skill and the project `AGENTS.md` section.
///
/// # Errors
///
/// Returns `HooksError::Io` on filesystem failures.
pub fn amp_uninstall(project_dir: &std::path::Path) -> Result<String, HooksError> {
    let mut msgs: Vec<String> = Vec::new();
    let (_skill_content, rel) = skill_for("amp", false)?;
    let skill_dst = dirs_home().join(rel);
    if skill_dst.exists() {
        fs::remove_file(&skill_dst)?;
        msgs.push("skill removed".to_string());
    }
    msgs.push(crate::platform::agents::agents_uninstall(
        project_dir,
        "amp",
    )?);
    Ok(msgs.join("\n"))
}

/// Return the top-level project-relative directory the install touches —
/// e.g. `.claude/skills/graphify/SKILL.md` → `.claude`. Used in the
/// `git add` hint printed after a project install.
fn scope_root_for(rel: &str) -> &str {
    // `rel` is fed from the string literals returned by `skill_for`, which
    // all contain `/`, so `.split('/').next()` is always `Some`. The
    // `unwrap_or(rel)` branch is purely defensive against a future caller
    // passing a path without a separator.
    rel.split('/').next().unwrap_or(rel)
}

/// Remove the auto-registered `## graphify` section from a CLAUDE.md
/// blob, returning the cleaned text. Tolerates either form (multi-line
/// section after the heading, or single-line registration). Preserves
/// the original trailing newline so well-formed files stay well-formed.
#[must_use]
fn strip_graphify_section(content: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("## ") {
            // Mirror the install side's exact-header check: only an
            // `## graphify` line opens a removable section. A docs
            // heading like `## How graphify works` must NOT be stripped.
            in_section = trimmed == "## graphify";
            if in_section {
                continue;
            }
        }
        if in_section {
            continue;
        }
        out.push(line);
    }
    let result = out.join("\n");
    if content.ends_with('\n') && !result.is_empty() {
        format!("{result}\n")
    } else {
        result
    }
}
