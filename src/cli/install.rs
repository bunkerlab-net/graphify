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
    gemini_install, gemini_uninstall, install_platform_skill, kiro_install, kiro_uninstall,
    pi_install, pi_uninstall, uninstall_all, vscode_install, vscode_uninstall,
};

use crate::PlatformCmd;

/// Install the graphify skill for the given platform.
///
/// Wraps `graphify_hooks::platform::install_platform_skill` and prints the
/// confirmation message returned by that function.
pub(crate) fn cmd_install(platform: &str) -> Result<()> {
    let msg = install_platform_skill(platform)?;
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
/// `agents_install` / `agents_uninstall`.
pub(crate) fn cmd_platform(platform: &str, cmd: &PlatformCmd) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let msg = match (platform, cmd) {
        ("claude", PlatformCmd::Install) => claude_install(&cwd)?,
        ("claude", PlatformCmd::Uninstall) => claude_uninstall(&cwd)?,
        ("gemini", PlatformCmd::Install) => gemini_install(&cwd)?,
        ("gemini", PlatformCmd::Uninstall) => gemini_uninstall(&cwd)?,
        ("vscode", PlatformCmd::Install) => vscode_install(&cwd)?,
        ("vscode", PlatformCmd::Uninstall) => vscode_uninstall(&cwd)?,
        ("copilot", PlatformCmd::Install) => copilot_install()?,
        ("copilot", PlatformCmd::Uninstall) => copilot_uninstall()?,
        ("kiro", PlatformCmd::Install) => kiro_install(&cwd)?,
        ("kiro", PlatformCmd::Uninstall) => kiro_uninstall(&cwd)?,
        ("pi", PlatformCmd::Install) => pi_install()?,
        ("pi", PlatformCmd::Uninstall) => pi_uninstall()?,
        ("antigravity", PlatformCmd::Install) => antigravity_install(&cwd)?,
        ("antigravity", PlatformCmd::Uninstall) => antigravity_uninstall(&cwd)?,
        ("cursor", PlatformCmd::Install) => cursor_install(&cwd)?,
        ("cursor", PlatformCmd::Uninstall) => cursor_uninstall(&cwd)?,
        (p, PlatformCmd::Install) => agents_install(&cwd, p)?,
        (p, PlatformCmd::Uninstall) => agents_uninstall(&cwd, p)?,
    };
    println!("{msg}");
    Ok(())
}
