//! Multi-platform uninstaller that drives every per-platform uninstall in
//! sequence.

use std::fs;
use std::path::Path;

use crate::HooksError;

/// Remove graphify from every platform detected in the current project.
///
/// Drives the per-platform `*_uninstall` and `uninstall_*` functions in
/// sequence, collecting their human-readable status strings into a single
/// report. Non-fatal errors are appended to the report as `warning: ...`.
///
/// When `purge` is true, also removes the `graphify-out/` directory.
///
/// # Errors
///
/// Returns `HooksError::Io` if a hard filesystem failure prevents reporting
/// (e.g. `graphify-out/` cannot be removed during `--purge`).
pub fn uninstall_all(project_dir: &Path, purge: bool) -> Result<String, HooksError> {
    use crate::platform::{
        agents::agents_uninstall, antigravity::antigravity_uninstall, claude::claude_uninstall,
        codebuddy::codebuddy_uninstall, codex::uninstall_codex_hook, cursor::cursor_uninstall,
        gemini::gemini_uninstall, kiro::kiro_uninstall, opencode::uninstall_opencode_plugin,
        vscode::vscode_uninstall,
    };

    let mut msgs = vec!["Uninstalling graphify from all detected platforms...\n".to_string()];

    let steps: Vec<Result<String, HooksError>> = vec![
        claude_uninstall(project_dir),
        codebuddy_uninstall(project_dir),
        gemini_uninstall(project_dir),
        vscode_uninstall(project_dir),
        cursor_uninstall(project_dir),
        kiro_uninstall(project_dir),
        antigravity_uninstall(project_dir, false),
        // AGENTS.md covers codex, aider, opencode, claw, droid, trae, trae-cn, hermes
        agents_uninstall(project_dir, ""),
        uninstall_opencode_plugin(project_dir),
        uninstall_codex_hook(project_dir),
    ];

    // The generic `agents` platform skill (#1432) lives at ~/.agents/skills
    // (global) and ./.agents/skills (project); its AGENTS.md section is handled
    // by the `agents_uninstall` step above. Remove both skill copies.
    super::fs::remove_skill(&super::fs::dirs_home().join(".agents/skills/graphify/SKILL.md"));
    super::fs::remove_skill(&project_dir.join(".agents/skills/graphify/SKILL.md"));

    for step in steps {
        match step {
            Ok(msg) if !msg.is_empty() => msgs.push(msg),
            Ok(_) => {}
            Err(e) => msgs.push(format!("  warning: {e}")),
        }
    }

    if purge {
        let out = project_dir.join("graphify-out");
        if out.exists() {
            fs::remove_dir_all(&out)?;
            msgs.push("\n  graphify-out/  ->  deleted (--purge)".to_string());
        } else {
            msgs.push("\n  graphify-out/  ->  not found (nothing to purge)".to_string());
        }
    }

    msgs.push("\nDone. Run 'cargo uninstall graphify' to remove the package itself.".to_string());
    Ok(msgs.join("\n"))
}
