//! Platform install/uninstall commands — delegates to `graphify-hooks`.
//!
//! Covers `install`, `uninstall`, and all per-platform subcommands
//! (`claude`, `gemini`, `cursor`, `vscode`, `copilot`, `kiro`, `pi`,
//! `antigravity`, `codex`, `opencode`, `aider`, `claw`, `droid`, `trae`,
//! `trae-cn`, `hermes`).

use anyhow::Result;
use graphify_hooks::platform::{
    agents_install, agents_uninstall, antigravity_install, antigravity_uninstall, claude_install,
    claude_uninstall, copilot_install, copilot_uninstall, cursor_install, cursor_uninstall,
    devin_install, devin_project_install, devin_project_uninstall, devin_uninstall, gemini_install,
    gemini_uninstall, install_platform_skill, install_platform_skill_project, kiro_install,
    kiro_uninstall, pi_install, pi_uninstall, uninstall_all, uninstall_platform_skill_project,
    vscode_install, vscode_uninstall,
};

use crate::cli::args::PlatformCmd;

/// Install the graphify skill for the given platform.
///
/// When `project` is `true`, installs into `./.{platform}/skills/...`
/// under the current working directory instead of the user home
/// directory. Mirrors the Python `--project` flag (#931).
pub(crate) fn cmd_install(platform: &str, project: bool) -> Result<()> {
    let msg = if project {
        let cwd = std::env::current_dir()?;
        install_platform_skill_project(platform, &cwd)?
    } else {
        install_platform_skill(platform)?
    };
    println!("{msg}");
    Ok(())
}

/// Remove graphify from all detected platforms in the current directory.
///
/// When `purge` is `true`, also deletes the `graphify-out/` directory.
pub(crate) fn cmd_uninstall(purge: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let msg = uninstall_all(&cwd, purge)?;
    println!("{msg}");
    Ok(())
}

/// Install or uninstall the graphify skill for a specific named platform.
///
/// Dispatches to the platform-specific `*_install` / `*_uninstall` functions
/// in `graphify_hooks::platform`. Unknown platform names fall through to
/// `agents_install` / `agents_uninstall`. When the `project` flag is set
/// on either subcommand, the install scope is the current working directory
/// rather than the home directory.
pub(crate) fn cmd_platform(platform: &str, cmd: &PlatformCmd) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let msg = match (platform, cmd) {
        // Devin has bespoke project-scope handling (writes .windsurf/rules),
        // so its `--project` branch is matched before the generic one.
        ("devin", PlatformCmd::Install { project: true }) => devin_project_install(&cwd)?,
        ("devin", PlatformCmd::Uninstall { project: true }) => devin_project_uninstall(&cwd)?,
        ("devin", PlatformCmd::Install { .. }) => devin_install()?,
        ("devin", PlatformCmd::Uninstall { .. }) => devin_uninstall()?,
        // Antigravity has bespoke project-scope handling: a `--project` install
        // writes only the workspace-local skill (rules + workflow are global-only;
        // `antigravity_install` early-returns for projects). Matched before the
        // generic `--project` branch.
        ("antigravity", PlatformCmd::Install { project: true }) => antigravity_install(&cwd, true)?,
        ("antigravity", PlatformCmd::Uninstall { project: true }) => {
            antigravity_uninstall(&cwd, true)?
        }
        (p, PlatformCmd::Install { project: true }) => install_platform_skill_project(p, &cwd)?,
        (p, PlatformCmd::Uninstall { project: true }) => uninstall_platform_skill_project(p, &cwd)?,
        ("claude", PlatformCmd::Install { .. }) => claude_install(&cwd)?,
        ("claude", PlatformCmd::Uninstall { .. }) => claude_uninstall(&cwd)?,
        ("gemini", PlatformCmd::Install { .. }) => gemini_install(&cwd)?,
        ("gemini", PlatformCmd::Uninstall { .. }) => gemini_uninstall(&cwd)?,
        ("vscode", PlatformCmd::Install { .. }) => vscode_install(&cwd)?,
        ("vscode", PlatformCmd::Uninstall { .. }) => vscode_uninstall(&cwd)?,
        ("copilot", PlatformCmd::Install { .. }) => copilot_install()?,
        ("copilot", PlatformCmd::Uninstall { .. }) => copilot_uninstall()?,
        ("kiro", PlatformCmd::Install { .. }) => kiro_install(&cwd)?,
        ("kiro", PlatformCmd::Uninstall { .. }) => kiro_uninstall(&cwd)?,
        ("pi", PlatformCmd::Install { .. }) => pi_install()?,
        ("pi", PlatformCmd::Uninstall { .. }) => pi_uninstall()?,
        ("antigravity", PlatformCmd::Install { .. }) => antigravity_install(&cwd, false)?,
        ("antigravity", PlatformCmd::Uninstall { .. }) => antigravity_uninstall(&cwd, false)?,
        ("cursor", PlatformCmd::Install { .. }) => cursor_install(&cwd)?,
        ("cursor", PlatformCmd::Uninstall { .. }) => cursor_uninstall(&cwd)?,
        (p, PlatformCmd::Install { .. }) => agents_install(&cwd, p)?,
        (p, PlatformCmd::Uninstall { .. }) => agents_uninstall(&cwd, p)?,
    };
    println!("{msg}");
    Ok(())
}
