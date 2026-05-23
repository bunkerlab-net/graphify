//! Compile-time-embedded skill markdown files from the `graphify-py`
//! submodule.
//!
//! Each constant is the verbatim content of the corresponding `skill-*.md`
//! file in the Python reference, embedded via `include_str!` so the
//! binaries don't need filesystem access to install skills.

pub(in crate::platform) const SKILL_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill.md");
pub(in crate::platform) const SKILL_CODEX_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-codex.md");
pub(in crate::platform) const SKILL_OPENCODE_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-opencode.md");
pub(in crate::platform) const SKILL_AIDER_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-aider.md");
pub(in crate::platform) const SKILL_COPILOT_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-copilot.md");
pub(in crate::platform) const SKILL_CLAW_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-claw.md");
pub(in crate::platform) const SKILL_DROID_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-droid.md");
pub(in crate::platform) const SKILL_TRAE_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-trae.md");
pub(in crate::platform) const SKILL_KIRO_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-kiro.md");
pub(in crate::platform) const SKILL_PI_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-pi.md");
pub(in crate::platform) const SKILL_WINDOWS_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-windows.md");
pub(in crate::platform) const SKILL_VSCODE_MD: &str =
    include_str!("../../../../../graphify-py/graphify/skill-vscode.md");

/// Skill registration text appended to `~/.claude/CLAUDE.md`.
pub(in crate::platform) const SKILL_REGISTRATION: &str = "\n# graphify\n\
- **graphify** (`~/.claude/skills/graphify/SKILL.md`) \
- any input to knowledge graph. Trigger: `/graphify`\n\
When the user types `/graphify`, invoke the Skill tool \
with `skill: \"graphify\"` before doing anything else.\n";
