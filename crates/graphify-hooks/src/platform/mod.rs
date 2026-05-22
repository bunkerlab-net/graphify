//! Per-platform install/uninstall functions for graphify.
//!
//! Each sub-module owns exactly one platform's logic. `common` holds shared
//! helpers and the two multi-platform entry points (`install_platform_skill`,
//! `uninstall_all`). This `mod.rs` is purely structural — no logic lives here.

pub mod agents;
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod common;
pub mod copilot;
pub mod cursor;
pub mod gemini;
pub mod kiro;
pub mod opencode;
pub mod pi;
pub mod vscode;

// Re-export the full public surface so callers can use `graphify_hooks::platform::*`
// as they did when everything lived in a single file.
pub use agents::{agents_install, agents_uninstall};
pub use antigravity::{antigravity_install, antigravity_uninstall};
pub use claude::{claude_install, claude_uninstall, install_claude_hook, uninstall_claude_hook};
pub use codex::{install_codex_hook, uninstall_codex_hook};
pub use common::{
    AGENTS_MD_SECTION, ANTIGRAVITY_RULES, ANTIGRAVITY_WORKFLOW, CLAUDE_MD_MARKER,
    CLAUDE_MD_SECTION, CURSOR_RULE, GEMINI_MD_SECTION, KIRO_STEERING, OPENCODE_PLUGIN_JS,
    SETTINGS_HOOK_MATCHER, VSCODE_INSTRUCTIONS_SECTION, install_platform_skill,
    replace_or_append_section, resolve_graphify_exe, uninstall_all,
};
pub use copilot::{copilot_install, copilot_uninstall};
pub use cursor::{cursor_install, cursor_uninstall};
pub use gemini::{gemini_install, gemini_uninstall, install_gemini_hook, uninstall_gemini_hook};
pub use kiro::{kiro_install, kiro_uninstall};
pub use opencode::{install_opencode_plugin, uninstall_opencode_plugin};
pub use pi::{pi_install, pi_uninstall};
pub use vscode::{vscode_install, vscode_uninstall};
