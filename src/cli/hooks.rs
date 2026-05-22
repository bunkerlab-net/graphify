//! `hook` subcommands — install, uninstall, and status for git hooks.

use anyhow::Result;

use crate::HookCmd;

/// Install, uninstall, or query the status of git hooks.
///
/// Delegates to `graphify_hooks::install` / `uninstall` / `status`
/// in the current working directory. Mirrors `__main__.py`'s `hook` command.
pub(crate) fn cmd_hook(cmd: &HookCmd) -> Result<()> {
    let cwd = std::env::current_dir()?;
    match cmd {
        HookCmd::Install => {
            let msg = graphify_hooks::install(&cwd)?;
            println!("{msg}");
        }
        HookCmd::Uninstall => {
            let msg = graphify_hooks::uninstall(&cwd)?;
            println!("{msg}");
        }
        HookCmd::Status => {
            println!("{}", graphify_hooks::status(&cwd));
        }
    }
    Ok(())
}
