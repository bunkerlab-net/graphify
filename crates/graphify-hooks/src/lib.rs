//! Git hook and AI-platform integration for graphify.
//!
//! Provides three categories of functionality:
//!
//! 1. **Git hooks** — install, uninstall, and report the status of graphify's
//!    `post-commit` and `post-checkout` hooks, which trigger background graph
//!    rebuilds without blocking the shell.
//!
//! 2. **Platform integrations** — install or remove graphify context files and
//!    tool-call hooks for every supported AI coding assistant:
//!    Claude Code, Codex, GitHub Copilot (CLI and VS Code), Cursor, Gemini CLI,
//!    Kiro, `OpenCode`, Pi, Antigravity, and the generic AGENTS.md platforms
//!    (aider, claw, droid, trae, trae-cn, hermes).
//!
//! 3. **Skill installers** — copy the embedded graphify skill markdown to the
//!    per-platform home-directory locations so assistants can invoke
//!    `/graphify` directly.
//!
//! Ports `graphify-py/graphify/hooks.py` and the platform install functions
//! from `graphify-py/graphify/__main__.py`.

mod constants;
mod error;
mod git;
mod install;
pub mod platform;
mod status;
mod uninstall;

pub use constants::{
    CHECKOUT_MARKER, CHECKOUT_MARKER_END, CHECKOUT_SCRIPT, HOOK_MARKER, HOOK_MARKER_END,
    HOOK_SCRIPT, PYTHON_DETECT,
};
pub use error::HooksError;
pub use git::{hooks_dir, hooks_dir_with, user_hooks_dir};
pub use install::install;
pub use platform::{
    check_skill_versions, refresh_all_version_stamps, skill_destinations, skill_version_warnings,
    user_skill_destinations, version_tuple,
};
pub use status::status;
pub use uninstall::uninstall;
