//! Cross-platform helpers and multi-platform entry points
//! (`install_platform_skill`, `uninstall_all`).
//!
//! This module centralises everything shared between per-platform
//! installers: embedded skill files, markdown constants, hook JSON
//! builders, atomic filesystem helpers.
//!
//! Sub-modules (all internal):
//! - `skills` — `include_str!`-embedded skill markdown files.
//! - `markdown` — section text written into CLAUDE.md / AGENTS.md / etc.
//! - `hooks_json` — Claude Code & Gemini hook JSON builders.
//! - `fs` — atomic write, JSON read/write, skill install/remove helpers.
//! - `install_skill` — multi-platform [`install_platform_skill`] entry point.
//! - `uninstall_all` — multi-platform [`uninstall_all`] entry point.

pub(super) mod fs;
pub(super) mod hooks_json;
pub(super) mod install_skill;
pub(super) mod markdown;
pub(super) mod skills;
pub(super) mod uninstall_all;

// Re-export the platform-public surface so existing `super::common::{...}`
// imports inside the per-platform files continue to resolve.
pub use fs::{replace_or_append_section, resolve_graphify_exe};
pub use install_skill::{
    install_platform_skill, install_platform_skill_project, uninstall_platform_skill_project,
};
pub use markdown::{
    AGENTS_MD_SECTION, ANTIGRAVITY_RULES, ANTIGRAVITY_WORKFLOW, CLAUDE_MD_MARKER,
    CLAUDE_MD_SECTION, CURSOR_RULE, GEMINI_MD_SECTION, KILO_PLUGIN_JS, KIRO_STEERING,
    OPENCODE_PLUGIN_JS, READ_SETTINGS_HOOK_MATCHER, SETTINGS_HOOK_MATCHER,
    VSCODE_INSTRUCTIONS_SECTION,
};
pub use uninstall_all::uninstall_all;

// Internal (platform-only) re-exports so per-platform files can keep using
// `super::common::{install_skill, dirs_home, ...}`.
pub(super) use fs::{
    dirs_home, install_skill, read_json_or_empty, remove_graphify_section, remove_skill, write_json,
};
pub(super) use hooks_json::{gemini_hook, read_settings_hook, settings_hook};
pub(super) use skills::{
    COMMAND_KILO_MD, SKILL_COPILOT_MD, SKILL_KIRO_MD, SKILL_MD, SKILL_PI_MD, SKILL_VSCODE_MD,
};
