//! Platform install/uninstall commands — delegates to `graphify-hooks`.
//!
//! Covers `install`, `uninstall`, and all per-platform subcommands
//! (`claude`, `gemini`, `cursor`, `vscode`, `copilot`, `kiro`, `pi`,
//! `antigravity`, `codex`, `opencode`, `aider`, `claw`, `droid`, `trae`,
//! `trae-cn`, `hermes`).

use std::io::IsTerminal;

use anyhow::Result;
use graphify_hooks::platform::{
    agents_install, agents_platform_install, agents_platform_uninstall, agents_uninstall,
    amp_install, amp_uninstall, antigravity_install, antigravity_uninstall, claude_install,
    claude_uninstall, codebuddy_install, codebuddy_uninstall, copilot_install, copilot_uninstall,
    cursor_install, cursor_uninstall, devin_install, devin_project_install,
    devin_project_uninstall, devin_uninstall, gemini_install, gemini_uninstall,
    install_kilo_skill_and_command, install_platform_skill, install_platform_skill_project,
    kilo_install, kilo_uninstall, kiro_install, kiro_uninstall, pi_install, pi_uninstall,
    uninstall_all, uninstall_platform_skill_project, vscode_install, vscode_uninstall,
};

use crate::cli::args::PlatformCmd;

/// Resolve a CLI platform alias to its canonical platform name. `skills` is the
/// friendly alias for the generic `agents` platform (#1432). Mirrors Python
/// `_canonical_platform`.
#[must_use]
pub(crate) fn canonical_platform(platform: &str) -> &str {
    if platform == "skills" {
        "agents"
    } else {
        platform
    }
}

/// `graphify agents install|uninstall` (and the `skills` alias): the amp-twin
/// of the generic Agent-Skills target — skill at `~/.agents/skills` plus the
/// always-on `AGENTS.md` section (#1432).
pub(crate) fn cmd_agents_platform(cmd: &PlatformCmd) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let msg = match cmd {
        PlatformCmd::Install { .. } => agents_platform_install(&cwd)?,
        PlatformCmd::Uninstall { .. } => agents_platform_uninstall(&cwd)?,
    };
    println!("{msg}");
    Ok(())
}
/// Print the amber-brain banner shown at the top of `graphify install`.
///
/// TTY-only (suppressed in CI logs and pipes) and best-effort — it never fails
/// the install. Mirrors graphify-py's `_print_banner`. Unlike Python it does
/// not toggle the Windows console mode via the OS API: that would pull in a
/// `winapi` dependency for a purely cosmetic banner, and modern Windows
/// terminals render ANSI by default. The banner is suppressed entirely when
/// stdout is not a terminal, so non-interactive runs are unaffected.
fn print_banner() {
    const AMBER: &str = "\x1b[38;5;214m";
    const DARK: &str = "\x1b[38;5;130m";
    const RESET: &str = "\x1b[0m";
    if !std::io::stdout().is_terminal() {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "{AMBER}
  ╭──◉──╮     ╭──◉──╮
 ╱  ◉   ◉ ╲ ╱ ◉   ◉  ╲
│   ◉─◉─◉  ◉  ◉─◉─◉   │
│    ◉   ◉ │ ◉   ◉    │
│   ◉─◉─◉  ◉  ◉─◉─◉   │
 ╲  ◉   ◉ ╱ ╲ ◉   ◉  ╱
  ╰──◉──╯     ╰──◉──╯
           ◉

  █▀▀ █▀█ ▄▀█ █▀█ █ █ █ █▀▀ █▄█
  █▄█ █▀▄ █▀█ █▀▀ █▀█ █ █▀   █{DARK}  {version}{RESET}
"
    );
}

/// Install the graphify skill for the given platform.
///
/// When `project` is `true`, installs into `./.{platform}/skills/...`
/// under the current working directory instead of the user home
/// directory. Mirrors the Python `--project` flag (#931).
pub(crate) fn cmd_install(platform: &str, project: bool) -> Result<()> {
    // Resolve `skills` -> `agents` before routing (#1432).
    let platform = canonical_platform(platform);
    print_banner();
    // Antigravity's project install lays down the full always-on layer
    // (skill + rules + workflow), not just the skill — matching graphify-py's
    // `_project_install("antigravity")`. The generic skill-only installer would
    // orphan the rules/workflow that uninstall removes.
    if project && platform == "antigravity" {
        let cwd = std::env::current_dir()?;
        let msg = antigravity_install(&cwd, true)?;
        println!("{msg}");
        return Ok(());
    }
    // Kilo's skill + `/graphify` command are global artefacts (the generic
    // skill installer doesn't know the command file), matching graphify-py's
    // `install(platform="kilo")`. The always-on project wiring is installed by
    // `graphify kilo install` (see `cmd_kilo`).
    if platform == "kilo" {
        let msg = install_kilo_skill_and_command()?;
        println!("{msg}");
        return Ok(());
    }
    let msg = if project {
        let cwd = std::env::current_dir()?;
        install_platform_skill_project(platform, &cwd)?
    } else {
        install_platform_skill(platform)?
    };
    println!("{msg}");
    Ok(())
}

/// Install or uninstall the full Kilo Code integration (#512).
///
/// `kilo install` writes the native skill + `/graphify` command globally and the
/// always-on `AGENTS.md` + `.kilo` plugin under the current project; `kilo
/// uninstall` reverses both. Mirrors graphify-py's `_kilo_install` /
/// `_kilo_uninstall`.
pub(crate) fn cmd_kilo(cmd: &PlatformCmd) -> Result<()> {
    let cwd = std::env::current_dir()?;
    // The shared `PlatformCmd::project` flag is intentionally ignored for Kilo:
    // graphify-py's `kilo` command (`_kilo_install` / `_kilo_uninstall`) has no
    // project-scope variant — the skill + command are always global and the
    // `.kilo` plugin is always written under the current working directory.
    // (CodeRabbit suggested branching on `project`; declined for parity.)
    let msg = match cmd {
        PlatformCmd::Install { .. } => kilo_install(&cwd)?,
        PlatformCmd::Uninstall { .. } => kilo_uninstall(&cwd)?,
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
        // writes the full always-on layer (skill + rules + workflow) under the
        // workspace, and `antigravity_install` prints the global-only MCP hint
        // only for non-project installs. Matched before the generic `--project`
        // branch.
        ("antigravity", PlatformCmd::Install { project: true }) => antigravity_install(&cwd, true)?,
        ("antigravity", PlatformCmd::Uninstall { project: true }) => {
            antigravity_uninstall(&cwd, true)?
        }
        // `CodeBuddy` writes CODEBUDDY.md + a .codebuddy/settings.json hook and
        // copies the skill, like `claude install` (#1136). graphify-py's
        // `codebuddy` CLI dispatch ignores `--project` (`codebuddy_install()` /
        // `codebuddy_uninstall()` at `__main__.py:2374-2377`), so BOTH the plain
        // and `--project` forms run the full CodeBuddy setup. Matched before the
        // generic `--project` branch below so the flag can't divert it to a
        // skill-only project install.
        ("codebuddy", PlatformCmd::Install { .. }) => codebuddy_install(&cwd)?,
        ("codebuddy", PlatformCmd::Uninstall { .. }) => codebuddy_uninstall(&cwd)?,
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
        // Amp's non-project install mirrors graphify-py `_amp_install`: clean the
        // legacy ~/.amp/skills dir, write the skill into ~/.config/agents/skills,
        // and write the AGENTS.md always-on section. Project scope flows through
        // the generic `install_platform_skill_project` arm above, which now wires
        // AGENTS.md for agents-group platforms.
        ("amp", PlatformCmd::Install { .. }) => amp_install(&cwd)?,
        ("amp", PlatformCmd::Uninstall { .. }) => amp_uninstall(&cwd)?,
        (p, PlatformCmd::Install { .. }) => agents_install(&cwd, p)?,
        (p, PlatformCmd::Uninstall { .. }) => agents_uninstall(&cwd, p)?,
    };
    println!("{msg}");
    Ok(())
}
