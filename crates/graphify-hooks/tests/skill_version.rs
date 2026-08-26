//! Parity tests against `graphify-py/tests/test_skill_version_warning.py`.
//!
//! Direction-aware skill-version mismatch warning (#1568): `_check_skill_version`
//! used to advise `graphify install` on ANY mismatch, but `install` re-stamps the
//! bundled (older) skill, so a NEWER on-disk skill would silently downgrade. The
//! warning is now direction-aware — skill-older recommends install, skill-newer
//! recommends upgrading the package.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use graphify_hooks::{
    skill_destinations, skill_version_warnings, user_skill_destinations, version_tuple,
};

/// Write a `SKILL.md` + `.graphify_version` stamp, returning the SKILL.md path.
fn make_skill(root: &Path, stamped: &str) -> PathBuf {
    let dir = root.join("skills").join("graphify");
    fs::create_dir_all(&dir).expect("mkdir skill dir");
    let skill_dst = dir.join("SKILL.md");
    fs::write(&skill_dst, "# graphify skill\n").expect("write skill");
    fs::write(dir.join(".graphify_version"), stamped).expect("write stamp");
    skill_dst
}

#[test]
fn version_tuple_orders_numerically() {
    assert!(version_tuple("0.9.2") > version_tuple("0.8.27")); // 9 > 8, not string-compared
    assert!(version_tuple("0.10.0") > version_tuple("0.9.0")); // 10 > 9
    assert_eq!(version_tuple("0.9.3"), version_tuple("0.9.3"));
    assert_eq!(version_tuple("1.0.0rc1"), version_tuple("1.0.0")); // suffix compares by core
    assert_eq!(version_tuple(""), vec![0]); // malformed stamp degrades, no panic
}

#[test]
fn skill_older_than_package_recommends_install() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let skill_dst = make_skill(tmp.path(), "0.8.27");
    let warnings = skill_version_warnings(&skill_dst, "0.9.3").join("\n");
    assert!(
        warnings.contains("Run 'graphify install' to update"),
        "got: {warnings}"
    );
    assert!(!warnings.contains("downgrade"), "got: {warnings}");
}

#[test]
fn skill_newer_than_package_recommends_upgrade_not_install() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let skill_dst = make_skill(tmp.path(), "0.9.2");
    let warnings = skill_version_warnings(&skill_dst, "0.8.27").join("\n");
    // Must NOT tell the user to run install (that would downgrade the skill).
    assert!(
        !warnings.contains("Run 'graphify install' to update"),
        "got: {warnings}"
    );
    assert!(warnings.contains("downgrade"), "got: {warnings}");
    assert!(
        warnings.to_lowercase().contains("upgrade"),
        "got: {warnings}"
    );
}

#[test]
fn matching_version_is_silent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let skill_dst = make_skill(tmp.path(), "0.9.3");
    assert_eq!(
        skill_version_warnings(&skill_dst, "0.9.3"),
        Vec::<String>::new()
    );
}

#[test]
fn missing_stamp_is_silent() {
    // No `.graphify_version` sibling -> nothing to check, no warning.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("skills").join("graphify");
    fs::create_dir_all(&dir).expect("mkdir");
    let skill_dst = dir.join("SKILL.md");
    fs::write(&skill_dst, "# skill\n").expect("write");
    assert_eq!(
        skill_version_warnings(&skill_dst, "0.9.3"),
        Vec::<String>::new()
    );
}

#[test]
fn missing_skill_with_stamp_warns_repair() {
    // Stamp present but SKILL.md gone -> repair warning.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("skills").join("graphify");
    fs::create_dir_all(&dir).expect("mkdir");
    fs::write(dir.join(".graphify_version"), "0.9.3").expect("stamp");
    let skill_dst = dir.join("SKILL.md");
    let warnings = skill_version_warnings(&skill_dst, "0.9.3").join("\n");
    assert!(warnings.contains("SKILL.md is missing"), "got: {warnings}");
}

#[test]
fn missing_references_sidecar_warns() {
    // A progressive SKILL.md that links references/ but has no sidecar dir.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("skills").join("graphify");
    fs::create_dir_all(&dir).expect("mkdir");
    let skill_dst = dir.join("SKILL.md");
    fs::write(&skill_dst, "see references/foo.md\n").expect("write");
    fs::write(dir.join(".graphify_version"), "0.9.3").expect("stamp");
    let warnings = skill_version_warnings(&skill_dst, "0.9.3").join("\n");
    assert!(
        warnings.contains("references/ sidecar is missing"),
        "got: {warnings}"
    );
}

/// Drift guard: the checked destination set must match the exact user-scope
/// paths graphify-py's `_platform_skill_destination` resolves, plus gemini
/// (which Python's `_PLATFORM_CONFIG` loop omits — a bug we deliberately fix).
#[test]
fn skill_destinations_match_expected_set() {
    let home = Path::new("/home/tester");
    let got: std::collections::BTreeSet<PathBuf> = skill_destinations(home, false, None, None)
        .into_iter()
        .collect();
    let expected: std::collections::BTreeSet<PathBuf> = [
        ".claude/skills",
        ".hermes/skills",
        ".gemini/skills",
        ".codex/skills",
        ".config/opencode/skills",
        ".config/kilo/skills",
        ".aider",
        ".copilot/skills",
        ".openclaw/skills",
        ".factory/skills",
        ".trae/skills",
        ".trae-cn/skills",
        ".kiro/skills",
        ".pi/agent/skills",
        ".codebuddy/skills",
        ".gemini/config/skills",
        ".kimi/skills",
        ".config/agents/skills",
        ".agents/skills",
        ".config/devin/skills",
    ]
    .iter()
    .map(|rel| home.join(rel).join("graphify").join("SKILL.md"))
    .collect();
    assert_eq!(got, expected);
}

/// The `CLAUDE_CONFIG_DIR` override relocates claude's checked destination.
#[test]
fn claude_config_dir_override_applies() {
    let home = Path::new("/home/tester");
    let cfg = PathBuf::from("/custom/claude");
    let dests = skill_destinations(home, false, Some(cfg.clone()), None);
    let expected = cfg.join("skills").join("graphify").join("SKILL.md");
    assert!(
        dests.contains(&expected),
        "claude override missing: {dests:?}"
    );
    assert!(
        !dests.contains(
            &home
                .join(".claude")
                .join("skills")
                .join("graphify")
                .join("SKILL.md")
        ),
        "default claude path must be replaced by the override"
    );
}

/// A real invocation returns a non-empty, deduplicated set.
#[test]
fn user_skill_destinations_is_nonempty_and_deduped() {
    let dests = user_skill_destinations();
    assert_ne!(dests, Vec::<PathBuf>::new());
    let unique: std::collections::BTreeSet<&PathBuf> = dests.iter().collect();
    assert_eq!(
        unique.len(),
        dests.len(),
        "destinations must be deduplicated"
    );
}

/// Producer side (#1568): installing a skill writes the `.graphify_version`
/// stamp the startup check later reads. Guards against the low-level
/// `install_skill` stamp write silently disappearing.
#[test]
fn install_writes_version_stamp() {
    let dir = tempfile::tempdir().expect("tempdir");
    graphify_hooks::platform::install_platform_skill_project("claude", dir.path())
        .expect("install");
    let stamp = dir
        .path()
        .join(".claude")
        .join("skills")
        .join("graphify")
        .join(".graphify_version");
    let got = fs::read_to_string(&stamp).expect("stamp written");
    assert_eq!(got, env!("CARGO_PKG_VERSION"));
}
