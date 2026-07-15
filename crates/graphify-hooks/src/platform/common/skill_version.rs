//! Skill-version stamp handling (#1568).
//!
//! Every installed `SKILL.md` gets a sibling `.graphify_version` stamp so that,
//! on the next CLI startup, [`check_skill_versions`] can warn when the skill on
//! disk was written by a different graphify version than the one now running.
//! The warning is direction-aware: an OLDER skill advises `graphify install`
//! (which re-stamps the bundled skill), but a NEWER skill advises upgrading the
//! package — because `install` would otherwise silently DOWNGRADE it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::fs::{claude_config_dir, dirs_home};

/// Filename of the version stamp written beside each installed `SKILL.md`.
pub(in crate::platform) const VERSION_STAMP: &str = ".graphify_version";

/// Parse a version string into a comparable integer tuple
/// (`0.9.2` -> `[0, 9, 2]`).
///
/// Reads the leading digits of each dot-segment, so pre/post-release suffixes
/// (`1.0.0rc1`) compare by their numeric core. A non-numeric or empty segment
/// becomes `0`, so a malformed stamp degrades to a conservative comparison
/// rather than panicking. Mirrors graphify-py `_version_tuple` (#1568).
#[must_use]
pub fn version_tuple(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|segment| {
            let digits: String = segment.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

/// `LOCALAPPDATA` as a `PathBuf`, or `None` when unset/empty (Windows only).
fn localappdata() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Deduplicated user-scope `SKILL.md` destinations to check for a stale stamp.
///
/// Mirrors `{_platform_skill_destination(n) for n in _PLATFORM_CONFIG}` in
/// graphify-py: every platform's global skill path, with the `claude`/`windows`
/// `CLAUDE_CONFIG_DIR` override and the Windows `hermes` `%LOCALAPPDATA%` quirk.
#[must_use]
pub fn user_skill_destinations() -> Vec<PathBuf> {
    skill_destinations(
        &dirs_home(),
        cfg!(target_os = "windows"),
        claude_config_dir(),
        localappdata(),
    )
}

/// Home-relative segments BEFORE the trailing `graphify/SKILL.md` for the
/// platforms whose global skill dir is a plain home subpath (antigravity +
/// antigravity-windows collapse to one dir; `aider` has no `skills` segment,
/// `pi` nests under `agent`). claude/hermes/gemini are handled separately.
const HOME_RELATIVE_SKILL_MIDS: &[&[&str]] = &[
    &[".codex", "skills"],
    &[".config", "opencode", "skills"],
    &[".config", "kilo", "skills"],
    &[".aider"],
    &[".copilot", "skills"],
    &[".openclaw", "skills"],
    &[".factory", "skills"],
    &[".trae", "skills"],
    &[".trae-cn", "skills"],
    &[".kiro", "skills"],
    &[".pi", "agent", "skills"],
    &[".codebuddy", "skills"],
    &[".gemini", "config", "skills"],
    &[".kimi", "skills"],
    &[".config", "agents", "skills"],
    &[".agents", "skills"],
    &[".config", "devin", "skills"],
];

/// Compute the deduplicated set of user-scope skill destinations from an
/// explicit environment. Testable core of [`user_skill_destinations`].
#[must_use]
pub fn skill_destinations(
    home: &Path,
    is_windows: bool,
    claude_config: Option<PathBuf>,
    localappdata: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut set: BTreeSet<PathBuf> = BTreeSet::new();

    // claude + windows share `~/.claude` (or `$CLAUDE_CONFIG_DIR`).
    let claude_root = claude_config.unwrap_or_else(|| home.join(".claude"));
    set.insert(claude_root.join("skills").join("graphify").join("SKILL.md"));

    // hermes scans `%LOCALAPPDATA%\hermes\skills` on Windows, `~/.hermes` off it (#1403).
    let hermes_root = if is_windows {
        localappdata
            .unwrap_or_else(|| home.join("AppData").join("Local"))
            .join("hermes")
    } else {
        home.join(".hermes")
    };
    set.insert(hermes_root.join("skills").join("graphify").join("SKILL.md"));

    // gemini: `~/.gemini/skills` off Windows, `~/.agents/skills` on it. Deliberate
    // divergence from graphify-py — its `_PLATFORM_CONFIG` iteration OMITS gemini
    // (gemini has no config entry), so a stamped gemini skill is never checked
    // there. We stamp it on install, so we check it too (AGENTS.md: fix the bug).
    let gemini_root = if is_windows {
        home.join(".agents")
    } else {
        home.join(".gemini")
    };
    set.insert(gemini_root.join("skills").join("graphify").join("SKILL.md"));

    for mid in HOME_RELATIVE_SKILL_MIDS {
        let mut p = home.to_path_buf();
        for seg in *mid {
            p.push(seg);
        }
        p.push("graphify");
        p.push("SKILL.md");
        set.insert(p);
    }

    set.into_iter().collect()
}

/// Warnings for one skill destination's stamp (0–2). Mirrors graphify-py
/// `_check_skill_version`. Returns the messages instead of printing so callers
/// (and tests) control the output stream.
#[must_use]
pub fn skill_version_warnings(skill_dst: &Path, current: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(parent) = skill_dst.parent() else {
        return warnings;
    };
    let version_file = parent.join(VERSION_STAMP);
    if !version_file.exists() {
        return warnings;
    }
    if !skill_dst.exists() {
        warnings.push(
            "  warning: skill dir exists but SKILL.md is missing. \
             Run 'graphify install' to repair."
                .to_string(),
        );
        return warnings;
    }
    // A progressive SKILL.md links to its `references/` sidecar; flag a missing
    // one for repair (the body points at fragments that won't load).
    let body = std::fs::read_to_string(skill_dst).unwrap_or_default();
    if body.contains("references/") && !parent.join("references").exists() {
        warnings.push(
            "  warning: skill references/ sidecar is missing. \
             Run 'graphify install' to repair."
                .to_string(),
        );
    }
    let Ok(installed_raw) = std::fs::read_to_string(&version_file) else {
        return warnings;
    };
    let installed = installed_raw.trim();
    if installed != current {
        if version_tuple(installed) > version_tuple(current) {
            // The skill on disk is NEWER than the running package. `graphify
            // install` writes the package's OWN (older) bundled skill and
            // re-stamps the version, so the old "run install" advice would
            // silently DOWNGRADE it. Upgrade the package instead (#1568).
            // `graphifyy` (double-y) is the real PyPI distribution name (graphify-py
            // `pyproject.toml` `name = "graphifyy"`), NOT a typo: the upgrade command
            // must name the installable package. graphify-py `__main__.py` emits the
            // same string.
            warnings.push(format!(
                "  warning: skill is from graphify {installed}, but the package is \
                 {current} (older). Upgrade the package (e.g. 'uv tool upgrade graphifyy' \
                 or 'pip install -U graphifyy'); running 'graphify install' would \
                 downgrade the skill."
            ));
        } else {
            warnings.push(format!(
                "  warning: skill is from graphify {installed}, package is {current}. \
                 Run 'graphify install' to update."
            ));
        }
    }
    warnings
}

/// Warn on `stderr` for every user-scope skill whose stamp mismatches `current`.
///
/// Callers skip this for silent commands (`install`, `uninstall`, `hook-check`,
/// `hook-guard`). Divergence from graphify-py: the "SKILL.md is missing"
/// warning goes to `stderr` here (Python sends that one line to stdout), keeping
/// stdout reserved for structured output.
pub fn check_skill_versions(current: &str) {
    for dst in user_skill_destinations() {
        for warning in skill_version_warnings(&dst, current) {
            eprintln!("{warning}");
        }
    }
}

/// Re-stamp `.graphify_version` in every OTHER already-installed skill dir after
/// a successful install, so a platform installed in a prior version doesn't
/// keep warning about a stale stamp. Best-effort. Mirrors graphify-py
/// `_refresh_all_version_stamps`.
pub fn refresh_all_version_stamps(current: &str) {
    for dst in user_skill_destinations() {
        if dst.exists()
            && let Some(parent) = dst.parent()
        {
            let _ = std::fs::write(parent.join(VERSION_STAMP), current);
        }
    }
}
