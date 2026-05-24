//! Multi-platform skill installer used by every platform that only needs
//! a home-directory skill file (no project-local files).

use std::fs;
use std::path::PathBuf;

use crate::HooksError;

use super::fs::{dirs_home, install_skill};
use super::skills::{
    SKILL_AIDER_MD, SKILL_CLAW_MD, SKILL_CODEX_MD, SKILL_COPILOT_MD, SKILL_DROID_MD, SKILL_KIRO_MD,
    SKILL_MD, SKILL_OPENCODE_MD, SKILL_PI_MD, SKILL_REGISTRATION, SKILL_TRAE_MD, SKILL_WINDOWS_MD,
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
    let (skill_content, home_rel): (&str, &str) = match platform {
        "claude" | "windows" => {
            let skill = if platform == "windows" {
                SKILL_WINDOWS_MD
            } else {
                SKILL_MD
            };
            (skill, ".claude/skills/graphify/SKILL.md")
        }
        "codex" => (SKILL_CODEX_MD, ".agents/skills/graphify/SKILL.md"),
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
        "antigravity" => (SKILL_MD, ".agents/skills/graphify/SKILL.md"),
        "antigravity-windows" => (SKILL_WINDOWS_MD, ".agents/skills/graphify/SKILL.md"),
        other => return Err(HooksError::UnknownPlatform(other.to_string())),
    };

    let skill_dst = if matches!(platform, "claude" | "windows") {
        if let Ok(cfg_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            PathBuf::from(cfg_dir)
                .join("skills")
                .join("graphify")
                .join("SKILL.md")
        } else {
            dirs_home().join(home_rel)
        }
    } else {
        dirs_home().join(home_rel)
    };

    install_skill(skill_content, &skill_dst)?;
    let mut msgs = vec![format!("  skill installed  ->  {}", skill_dst.display())];

    if matches!(platform, "claude" | "windows") {
        let claude_md = if let Ok(cfg_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            PathBuf::from(cfg_dir).join("CLAUDE.md")
        } else {
            dirs_home().join(".claude").join("CLAUDE.md")
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

    msgs.push(String::new());
    msgs.push("Done. Open your AI coding assistant and type:".to_string());
    msgs.push(String::new());
    msgs.push("  /graphify .".to_string());
    msgs.push(String::new());
    Ok(msgs.join("\n"))
}
