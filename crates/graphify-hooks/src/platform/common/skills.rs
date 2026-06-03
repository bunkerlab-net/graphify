//! Compile-time-embedded skill markdown files shipped with the Rust port
//! of graphify.
//!
//! The Python distribution shipped one tailored skill per platform because
//! each host (Codex, Aider, Copilot, etc.) needed bespoke subagent-dispatch
//! syntax to drive the per-chunk semantic extraction. The Rust binary
//! handles the full pipeline (`graphify extract <PATH>`) without any
//! host-driven subagent fan-out, so every platform can share the same
//! canonical skill. VS Code keeps a thin variant because the GitHub Copilot
//! CLI surfaces a slightly different invocation model.

/// Canonical Rust-native graphify skill, embedded at compile time so the
/// installed binary needs no filesystem access to write the skill out.
pub(in crate::platform) const SKILL_MD: &str = include_str!("../../../skills/skill.md");

/// VS Code / GitHub Copilot variant — same pipeline, slightly different
/// invocation language to suit the chat-side UX.
pub(in crate::platform) const SKILL_VSCODE_MD: &str =
    include_str!("../../../skills/skill-vscode.md");

/// Kilo Code `/graphify` command file (`~/.config/kilo/command/graphify.md`).
///
/// Kilo supports a native slash-command that hands off to the graphify skill;
/// this is the command definition, byte-identical to graphify-py's
/// `command-kilo.md`.
pub(in crate::platform) const COMMAND_KILO_MD: &str =
    include_str!("../../../skills/command-kilo.md");

// All other platform variants reuse the canonical skill. The Python
// distribution shipped per-platform variants only to encode bespoke
// subagent-dispatch syntax — the Rust binary runs the whole pipeline as a
// single `graphify extract` call, so a single skill suffices everywhere.
pub(in crate::platform) const SKILL_CODEX_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_OPENCODE_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_AIDER_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_COPILOT_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_CLAW_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_DROID_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_TRAE_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_KIRO_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_PI_MD: &str = SKILL_MD;
pub(in crate::platform) const SKILL_WINDOWS_MD: &str = SKILL_MD;

/// Skill registration text appended to `~/.claude/CLAUDE.md`.
pub(in crate::platform) const SKILL_REGISTRATION: &str = "\n# graphify\n\
- **graphify** (`~/.claude/skills/graphify/SKILL.md`) \
- any input to knowledge graph. Trigger: `/graphify`\n\
When the user types `/graphify`, invoke the Skill tool \
with `skill: \"graphify\"` before doing anything else.\n";
