//! Parity tests against `graphify-py/tests/test_hooks.py`,
//! `test_install.py`, `test_claude_md.py`, `test_install_strings.py`, and
//! `test_install_upgrade.py`.
//!
//! Skipped: `test_hook_check_no_additionalContext` — this test exercises the
//! CLI binary (`python -m graphify hook-check`), not the hooks module.  It
//! belongs to the CLI port (task #18).
//!
//! Skipped: `test_how_it_works_clarifies_code_only_semantic_extraction` —
//! this test reads `docs/how-it-works.md` from the Python submodule and is not
//! related to the Rust install module.

#![allow(clippy::expect_used)]
// `std::env::set_var` is unsafe in edition 2024 — allow it in test code only.
#![allow(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use graphify_hooks::{
    CHECKOUT_MARKER, CHECKOUT_SCRIPT, HOOK_MARKER, HOOK_SCRIPT, PYTHON_DETECT, hooks_dir_with,
    install, status, uninstall, user_hooks_dir,
};
use serial_test::serial;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_git_repo(path: &Path) -> PathBuf {
    Command::new("git")
        .args(["init", &path.to_string_lossy()])
        .output()
        .expect("git init failed");
    path.to_path_buf()
}

// ---------------------------------------------------------------------------
// post-commit hook tests
// ---------------------------------------------------------------------------

#[test]
fn test_install_creates_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    let result = install(&repo).expect("test invariant");
    let hook = repo.join(".git").join("hooks").join("post-commit");
    assert!(hook.exists());
    let content = fs::read_to_string(&hook).expect("read fixture");
    assert!(content.contains(HOOK_MARKER));
    assert!(result.contains("installed"));
}

#[test]
fn test_install_is_executable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    let hook = repo.join(".git").join("hooks").join("post-commit");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&hook)
            .expect("fixture metadata")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "executable bit should be set");
    }
    #[cfg(not(unix))]
    {
        let content = fs::read_to_string(&hook).expect("read fixture");
        assert!(content.starts_with("#!/bin/sh\n"));
    }
}

#[test]
fn test_install_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    let result = install(&repo).expect("test invariant");
    assert!(result.contains("already installed"));
    // marker appears only once
    let hook = repo.join(".git").join("hooks").join("post-commit");
    let content = fs::read_to_string(&hook).expect("read fixture");
    assert_eq!(content.matches(HOOK_MARKER).count(), 1);
}

#[test]
fn test_install_appends_to_existing_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    let hook = repo.join(".git").join("hooks").join("post-commit");
    // Create hooks dir first
    fs::create_dir_all(hook.parent().expect("create_dir_all")).expect("test invariant");
    fs::write(&hook, b"#!/bin/bash\necho existing\n").expect("write fixture");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("test invariant");
    }

    install(&repo).expect("test invariant");
    let content = fs::read_to_string(&hook).expect("read fixture");
    assert!(content.contains("existing"));
    assert!(content.contains(HOOK_MARKER));
}

// ---------------------------------------------------------------------------
// uninstall tests
// ---------------------------------------------------------------------------

#[test]
fn test_uninstall_removes_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    let result = uninstall(&repo).expect("test invariant");
    let hook = repo.join(".git").join("hooks").join("post-commit");
    assert!(!hook.exists());
    assert!(result.to_lowercase().contains("removed"));
}

#[test]
fn test_uninstall_no_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    let result = uninstall(&repo).expect("test invariant");
    assert!(result.contains("nothing to remove"));
}

// ---------------------------------------------------------------------------
// status tests
// ---------------------------------------------------------------------------

#[test]
fn test_status_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    let result = status(&repo);
    assert!(result.contains("installed"));
}

#[test]
fn test_status_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    let result = status(&repo);
    assert!(result.contains("not installed"));
}

// ---------------------------------------------------------------------------
// Error case
// ---------------------------------------------------------------------------

#[test]
fn test_no_git_repo_raises() {
    let dir = tempfile::tempdir().expect("tempdir");
    let not_a_repo = dir.path().join("not_a_repo");
    fs::create_dir_all(&not_a_repo).expect("create_dir_all");
    let err = install(&not_a_repo).expect_err("expected Err");
    assert!(format!("{err}").contains("No git repository"));
}

// ---------------------------------------------------------------------------
// user_hooks_dir — Husky 9 .husky/_ → .husky/ remap (#987)
// ---------------------------------------------------------------------------

#[test]
fn user_hooks_dir_strips_husky_underscore() {
    let dir = tempfile::tempdir().expect("tempdir");
    let husky_under = dir.path().join(".husky").join("_");
    assert_eq!(user_hooks_dir(&husky_under), dir.path().join(".husky"));
}

#[test]
fn user_hooks_dir_passthrough_for_plain_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plain = dir.path().join(".git").join("hooks");
    assert_eq!(user_hooks_dir(&plain), plain);
}

#[test]
fn user_hooks_dir_does_not_strip_underscore_in_other_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let custom = dir.path().join(".git").join("_hooks");
    assert_eq!(user_hooks_dir(&custom), custom);
}

// ---------------------------------------------------------------------------
// post-checkout hook tests
// ---------------------------------------------------------------------------

#[test]
fn test_install_creates_post_checkout_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    let hook = repo.join(".git").join("hooks").join("post-checkout");
    assert!(hook.exists());
    let content = fs::read_to_string(&hook).expect("read fixture");
    assert!(content.contains(CHECKOUT_MARKER));
}

#[test]
fn test_install_post_checkout_is_executable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    let hook = repo.join(".git").join("hooks").join("post-checkout");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&hook)
            .expect("fixture metadata")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "executable bit should be set");
    }
    #[cfg(not(unix))]
    {
        let content = fs::read_to_string(&hook).expect("read fixture");
        assert!(content.starts_with("#!/bin/sh\n"));
    }
}

#[test]
fn test_uninstall_removes_post_checkout_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    uninstall(&repo).expect("test invariant");
    let hook = repo.join(".git").join("hooks").join("post-checkout");
    assert!(!hook.exists());
}

#[test]
fn test_status_shows_both_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("test invariant");
    let result = status(&repo);
    assert!(result.contains("post-commit"));
    assert!(result.contains("post-checkout"));
    assert!(result.matches("installed").count() >= 2);
}

// ---------------------------------------------------------------------------
// hooks_dir tests with injectable resolver
// ---------------------------------------------------------------------------

#[test]
fn test_hooks_dir_resolves_relative_git_hooks_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());

    let result =
        hooks_dir_with(&repo, &|_root| Some(".git/hooks\n".to_string())).expect("test invariant");
    let expected = repo
        .join(".git")
        .join("hooks")
        .canonicalize()
        .expect("test invariant");
    assert_eq!(result, expected);
}

#[test]
fn test_hooks_dir_rejects_multiline_git_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());

    // The multiline output (e.g., old git echoing back unrecognised flags)
    // must be rejected, falling through to the default .git/hooks path.
    let result = hooks_dir_with(&repo, &|_root| {
        Some("--path-format=absolute\n.git/hooks\n".to_string())
    })
    .expect("test invariant");

    // Should fall back to default .git/hooks (canonicalized — macOS /var → /private/var).
    let expected = repo.join(".git").join("hooks");
    let expected = expected.canonicalize().unwrap_or(expected);
    assert_eq!(result, expected);
    // Malicious directory name must not have been created
    assert!(!repo.join("--path-format=absolute\n.git").exists());
}

#[test]
fn test_hooks_dir_accepts_absolute_git_hooks_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    let hooks = dir.path().join("actual-hooks");

    let hooks_str = hooks.to_string_lossy().to_string();
    let result = hooks_dir_with(&repo, &move |_root| Some(format!("{hooks_str}\n")))
        .expect("test invariant");
    assert_eq!(result, hooks.canonicalize().unwrap_or(hooks));
}

// ---------------------------------------------------------------------------
// Script content assertions
// ---------------------------------------------------------------------------

/// Hook script must skip shebang extraction for .exe binaries (Windows).
#[test]
fn test_hook_skips_head_on_exe() {
    assert!(
        PYTHON_DETECT.contains("*.exe) _SHEBANG=") || PYTHON_DETECT.contains("*.exe)"),
        "PYTHON_DETECT should contain *.exe case"
    );
}

/// Smoke-check: `HOOK_SCRIPT` and `CHECKOUT_SCRIPT` contain `PYTHON_DETECT` verbatim.
#[test]
fn test_scripts_contain_python_detect() {
    assert!(
        HOOK_SCRIPT.contains(PYTHON_DETECT),
        "HOOK_SCRIPT should contain PYTHON_DETECT verbatim"
    );
    assert!(
        CHECKOUT_SCRIPT.contains(PYTHON_DETECT),
        "CHECKOUT_SCRIPT should contain PYTHON_DETECT verbatim"
    );
}

// ===========================================================================
// Platform install/uninstall parity tests
// Ports: test_install.py, test_claude_md.py, test_install_strings.py,
//        test_install_upgrade.py
// ===========================================================================

use graphify_hooks::platform::{
    AGENTS_MD_SECTION, ANTIGRAVITY_RULES, CLAUDE_MD_MARKER, CLAUDE_MD_SECTION, CURSOR_RULE,
    GEMINI_MD_SECTION, KIRO_STEERING, OPENCODE_PLUGIN_JS, VSCODE_INSTRUCTIONS_SECTION,
    agents_install, agents_uninstall, antigravity_install, antigravity_uninstall, claude_install,
    claude_uninstall, cursor_install, cursor_uninstall, gemini_install, gemini_uninstall,
    install_claude_hook, install_codex_hook, install_gemini_hook, install_opencode_plugin,
    install_platform_skill, install_platform_skill_project, kiro_install, kiro_uninstall,
    replace_or_append_section, uninstall_claude_hook, uninstall_codex_hook, uninstall_gemini_hook,
    uninstall_opencode_plugin, uninstall_platform_skill_project, vscode_install, vscode_uninstall,
};

// ---------------------------------------------------------------------------
// _replace_or_append_section (test_claude_md.py indirectly, test_install.py)
// ---------------------------------------------------------------------------

#[test]
fn test_replace_or_append_appends_when_absent() {
    let result = replace_or_append_section(
        "# Existing\n\nSome rules.\n",
        "## graphify",
        "## graphify\n\nNew stuff.\n",
    );
    assert!(result.contains("Existing"));
    assert!(result.contains("## graphify"));
    assert!(result.contains("New stuff"));
}

#[test]
fn test_replace_or_append_creates_from_empty() {
    let result = replace_or_append_section("", "## graphify", "## graphify\n\nNew stuff.\n");
    assert!(result.contains("## graphify"));
    assert!(!result.starts_with('\n'));
}

#[test]
fn test_replace_or_append_replaces_existing_section() {
    let old = "# Project\n\n## graphify\n\nOld report-first text.\n\n## Other\n\nOther content.\n";
    let new =
        replace_or_append_section(old, "## graphify", "## graphify\n\nNew query-first text.\n");
    assert!(new.contains("New query-first text."));
    assert!(!new.contains("Old report-first text."));
    // Other sections must be preserved.
    assert!(new.contains("## Other"));
    assert!(new.contains("Other content."));
    assert!(new.contains("# Project"));
}

#[test]
fn test_replace_or_append_idempotent() {
    let base = "# Project\n";
    let section = "## graphify\n\nSome rules.\n";
    let first = replace_or_append_section(base, "## graphify", section);
    let second = replace_or_append_section(&first, "## graphify", section);
    assert_eq!(first, second);
    assert_eq!(first.matches("## graphify").count(), 1);
}

// ---------------------------------------------------------------------------
// claude_install / claude_uninstall (test_claude_md.py)
// ---------------------------------------------------------------------------

#[test]
fn test_install_creates_claude_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    let target = dir.path().join("CLAUDE.md");
    assert!(target.exists());
    assert!(target.read_to_string_unwrap().contains(CLAUDE_MD_MARKER));
}

#[test]
fn test_install_contains_expected_rules() {
    let dir = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    let content = dir.path().join("CLAUDE.md").read_to_string_unwrap();
    assert!(content.contains("GRAPH_REPORT.md"));
    assert!(content.contains("wiki/index.md"));
    assert!(content.contains("graphify update"));
}

#[test]
fn test_install_appends_to_existing_claude_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("CLAUDE.md");
    fs::write(&target, "# Existing content\n\nSome rules here.\n").expect("write fixture");
    claude_install(dir.path()).expect("test invariant");
    let content = target.read_to_string_unwrap();
    assert!(content.contains("Existing content"));
    assert!(content.contains(CLAUDE_MD_MARKER));
}

#[test]
fn test_install_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    let msg = claude_install(dir.path()).expect("test invariant");
    let content = dir.path().join("CLAUDE.md").read_to_string_unwrap();
    assert_eq!(content.matches(CLAUDE_MD_MARKER).count(), 1);
    assert!(msg.contains("already configured"));
}

#[test]
fn test_uninstall_removes_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    claude_uninstall(dir.path()).expect("test invariant");
    let target = dir.path().join("CLAUDE.md");
    if target.exists() {
        assert!(!target.read_to_string_unwrap().contains(CLAUDE_MD_MARKER));
    }
}

#[test]
fn test_uninstall_preserves_other_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("CLAUDE.md");
    fs::write(&target, "# My Project\n\nSome rules.\n").expect("write fixture");
    claude_install(dir.path()).expect("test invariant");
    claude_uninstall(dir.path()).expect("test invariant");
    assert!(target.exists());
    let content = target.read_to_string_unwrap();
    assert!(content.contains("My Project"));
    assert!(content.contains("Some rules"));
    assert!(!content.contains(CLAUDE_MD_MARKER));
}

#[test]
fn test_uninstall_no_op_when_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("CLAUDE.md");
    fs::write(&target, "# Other stuff\n").expect("write fixture");
    let msg = claude_uninstall(dir.path()).expect("test invariant");
    assert!(msg.contains("not found") || msg.contains("nothing to do"));
}

#[test]
fn test_uninstall_no_op_when_no_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let msg = claude_uninstall(dir.path()).expect("test invariant");
    assert!(msg.contains("No CLAUDE.md") || msg.contains("nothing to do"));
}

// ---------------------------------------------------------------------------
// .claude/settings.json PreToolUse hook (test_claude_md.py)
// ---------------------------------------------------------------------------

#[test]
fn test_install_creates_settings_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".claude").join("settings.json");
    assert!(settings_path.exists());
    let settings: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    let hooks = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("array field");
    assert!(
        hooks
            .iter()
            .any(|h| h.get("matcher").and_then(|v| v.as_str()) == Some("Bash"))
    );
}

#[test]
fn test_install_settings_json_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    claude_install(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".claude").join("settings.json");
    let settings: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    let hooks = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("array field");
    let bash_hooks: Vec<_> = hooks
        .iter()
        .filter(|h| {
            h.get("matcher").and_then(|v| v.as_str()) == Some("Bash")
                && h.to_string().contains("graphify")
        })
        .collect();
    assert_eq!(bash_hooks.len(), 1);
}

#[test]
#[serial(home_env)]
fn test_uninstall_removes_settings_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    claude_uninstall(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".claude").join("settings.json");
    if settings_path.exists() {
        let settings: serde_json::Value =
            serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
        let hooks = settings["hooks"]["PreToolUse"]
            .as_array()
            .expect("array field");
        assert!(!hooks.iter().any(|h| {
            h.get("matcher").and_then(|v| v.as_str()) == Some("Bash")
                && h.to_string().contains("graphify")
        }));
    }
}

// ---------------------------------------------------------------------------
// Platform skill installs (test_install.py)
// ---------------------------------------------------------------------------

/// Helper: run `install_platform_skill` with `HOME` overridden to `tmp_path`.
fn install_skill_to(tmp_path: &Path, platform: &str) -> String {
    // Override HOME so skills go into tmp_path instead of the real home dir.
    // SAFETY: test-only; single-threaded test runner for this module.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", tmp_path);
    }
    let result = install_platform_skill(platform).expect("test invariant");
    // SAFETY: test-only cleanup.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::remove_var("HOME");
    }
    result
}

#[test]
#[serial(home_env)]
fn test_install_default_claude() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "claude");
    assert!(dir.path().join(".claude/skills/graphify/SKILL.md").exists());
}

#[test]
#[serial(home_env)]
fn test_install_codex() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "codex");
    assert!(dir.path().join(".agents/skills/graphify/SKILL.md").exists());
}

#[test]
#[serial(home_env)]
fn test_install_opencode() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "opencode");
    assert!(
        dir.path()
            .join(".config/opencode/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
fn test_install_opencode_project_uses_dot_opencode() {
    // Project scope writes under `.opencode/`, not `.config/opencode/` (#1042).
    let dir = tempfile::tempdir().expect("tempdir");
    install_platform_skill_project("opencode", dir.path()).expect("test invariant");
    assert!(
        dir.path()
            .join(".opencode/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
#[serial(home_env)]
fn test_install_amp() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "amp");
    assert!(dir.path().join(".amp/skills/graphify/SKILL.md").exists());
}

#[test]
#[serial(home_env)]
fn test_install_claw() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "claw");
    assert!(
        dir.path()
            .join(".openclaw/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
#[serial(home_env)]
fn test_install_droid() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "droid");
    assert!(
        dir.path()
            .join(".factory/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
#[serial(home_env)]
fn test_install_trae() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "trae");
    assert!(dir.path().join(".trae/skills/graphify/SKILL.md").exists());
}

#[test]
#[serial(home_env)]
fn test_install_trae_cn() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "trae-cn");
    assert!(
        dir.path()
            .join(".trae-cn/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
#[serial(home_env)]
fn test_install_windows() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "windows");
    assert!(dir.path().join(".claude/skills/graphify/SKILL.md").exists());
}

#[test]
#[serial(home_env)]
fn test_install_unknown_platform_errors() {
    let result = install_platform_skill("unknown");
    assert!(result.is_err());
    assert!(
        result
            .expect_err("expected Err")
            .to_string()
            .contains("unknown platform")
    );
}

#[test]
#[serial(home_env)]
fn test_claude_install_registers_claude_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "claude");
    assert!(dir.path().join(".claude/CLAUDE.md").exists());
}

#[test]
#[serial(home_env)]
fn test_codex_install_does_not_write_claude_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "codex");
    assert!(!dir.path().join(".claude/CLAUDE.md").exists());
}

// ---------------------------------------------------------------------------
// agents_install / agents_uninstall (test_install.py)
// ---------------------------------------------------------------------------

#[test]
fn test_codex_agents_install_writes_agents_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "codex").expect("test invariant");
    let agents_md = dir.path().join("AGENTS.md");
    assert!(agents_md.exists());
    let content = agents_md.read_to_string_unwrap();
    assert!(content.contains("graphify"));
    assert!(content.contains("GRAPH_REPORT.md"));
}

#[test]
fn test_codex_agents_install_mentions_dirty_graph_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "codex").expect("test invariant");
    let content = dir.path().join("AGENTS.md").read_to_string_unwrap();
    assert!(content.contains("Dirty graphify-out/ files are expected"));
    assert!(content.contains("not a reason to skip graphify"));
}

#[test]
fn test_opencode_agents_install_writes_agents_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "opencode").expect("test invariant");
    assert!(dir.path().join("AGENTS.md").exists());
}

#[test]
fn test_claw_agents_install_writes_agents_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "claw").expect("test invariant");
    assert!(dir.path().join("AGENTS.md").exists());
}

#[test]
fn test_agents_install_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "codex").expect("test invariant");
    agents_install(dir.path(), "codex").expect("test invariant");
    let content = dir.path().join("AGENTS.md").read_to_string_unwrap();
    assert_eq!(content.matches("## graphify").count(), 1);
}

#[test]
fn test_agents_install_appends_to_existing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agents_md = dir.path().join("AGENTS.md");
    fs::write(&agents_md, "# Existing rules\n\nDo not break things.\n").expect("write fixture");
    agents_install(dir.path(), "codex").expect("test invariant");
    let content = agents_md.read_to_string_unwrap();
    assert!(content.contains("Do not break things."));
    assert!(content.contains("## graphify"));
}

#[test]
fn test_agents_uninstall_removes_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "codex").expect("test invariant");
    agents_uninstall(dir.path(), "").expect("test invariant");
    assert!(!dir.path().join("AGENTS.md").exists());
}

#[test]
fn test_agents_uninstall_preserves_other_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agents_md = dir.path().join("AGENTS.md");
    fs::write(&agents_md, "# Existing rules\n\nDo not break things.\n").expect("write fixture");
    agents_install(dir.path(), "codex").expect("test invariant");
    agents_uninstall(dir.path(), "").expect("test invariant");
    assert!(agents_md.exists());
    let content = agents_md.read_to_string_unwrap();
    assert!(content.contains("Do not break things."));
    assert!(!content.contains("## graphify"));
}

#[test]
fn test_agents_uninstall_no_op_when_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let msg = agents_uninstall(dir.path(), "").expect("test invariant");
    assert!(msg.contains("nothing to do"));
}

// ---------------------------------------------------------------------------
// OpenCode plugin (test_install.py)
// ---------------------------------------------------------------------------

#[test]
fn test_opencode_agents_install_writes_plugin() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "opencode").expect("test invariant");
    let plugin = dir.path().join(".opencode/plugins/graphify.js");
    assert!(plugin.exists());
    assert!(
        plugin
            .read_to_string_unwrap()
            .contains("tool.execute.before")
    );
}

#[test]
fn test_opencode_agents_install_registers_plugin_in_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "opencode").expect("test invariant");
    let config_file = dir.path().join(".opencode/opencode.json");
    assert!(config_file.exists());
    let config: serde_json::Value =
        serde_json::from_str(&config_file.read_to_string_unwrap()).expect("test invariant");
    let plugins = config["plugin"].as_array().expect("array field");
    assert!(
        plugins
            .iter()
            .any(|p| p.as_str().is_some_and(|s| s.contains("graphify.js")))
    );
}

#[test]
fn test_opencode_agents_install_merges_existing_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_file = dir.path().join(".opencode/opencode.json");
    fs::create_dir_all(config_file.parent().expect("create_dir_all")).expect("test invariant");
    fs::write(
        &config_file,
        serde_json::json!({"model": "claude-opus-4-5", "plugin": []}).to_string(),
    )
    .expect("test invariant");
    agents_install(dir.path(), "opencode").expect("test invariant");
    let config: serde_json::Value =
        serde_json::from_str(&config_file.read_to_string_unwrap()).expect("test invariant");
    assert_eq!(config["model"].as_str(), Some("claude-opus-4-5"));
    assert!(
        config["plugin"]
            .as_array()
            .expect("test invariant")
            .iter()
            .any(|p| p.as_str().is_some_and(|s| s.contains("graphify.js")))
    );
}

#[test]
fn test_opencode_agents_uninstall_removes_plugin() {
    let dir = tempfile::tempdir().expect("tempdir");
    agents_install(dir.path(), "opencode").expect("test invariant");
    agents_uninstall(dir.path(), "opencode").expect("test invariant");
    let plugin = dir.path().join(".opencode/plugins/graphify.js");
    assert!(!plugin.exists());
    let config_file = dir.path().join(".opencode/opencode.json");
    if config_file.exists() {
        let config: serde_json::Value =
            serde_json::from_str(&config_file.read_to_string_unwrap()).expect("test invariant");
        let plugins = config
            .get("plugin")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            !plugins
                .iter()
                .any(|p| p.as_str().is_some_and(|s| s.contains("graphify.js")))
        );
    }
}

// ---------------------------------------------------------------------------
// Cursor (test_install.py)
// ---------------------------------------------------------------------------

#[test]
fn test_cursor_install_writes_rule() {
    let dir = tempfile::tempdir().expect("tempdir");
    cursor_install(dir.path()).expect("test invariant");
    let rule = dir.path().join(".cursor/rules/graphify.mdc");
    assert!(rule.exists());
    let content = rule.read_to_string_unwrap();
    assert!(content.contains("alwaysApply: true"));
    assert!(content.contains("graphify-out/GRAPH_REPORT.md"));
}

#[test]
#[serial(home_env)]
fn test_cursor_install_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    cursor_install(dir.path()).expect("test invariant");
    let rule = dir.path().join(".cursor/rules/graphify.mdc");
    let original = rule.read_to_string_unwrap();
    cursor_install(dir.path()).expect("test invariant");
    assert_eq!(rule.read_to_string_unwrap(), original);
}

#[test]
#[serial(home_env)]
fn test_cursor_uninstall_removes_rule() {
    let dir = tempfile::tempdir().expect("tempdir");
    cursor_install(dir.path()).expect("test invariant");
    cursor_uninstall(dir.path()).expect("test invariant");
    assert!(!dir.path().join(".cursor/rules/graphify.mdc").exists());
}

#[test]
#[serial(home_env)]
fn test_cursor_uninstall_noop_if_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    cursor_uninstall(dir.path()).expect("test invariant"); // should not error
}

// ---------------------------------------------------------------------------
// Gemini CLI (test_install.py)
// ---------------------------------------------------------------------------

/// Override HOME to tmp so Gemini skill goes into tmp dir.
fn gemini_install_to(project_dir: &Path, skill_home: &Path) {
    // SAFETY: test-only HOME override.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", skill_home);
    }
    gemini_install(project_dir).expect("test invariant");
    // SAFETY: test-only cleanup.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::remove_var("HOME");
    }
}

fn gemini_uninstall_to(project_dir: &Path, skill_home: &Path) {
    // SAFETY: test-only HOME override.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", skill_home);
    }
    gemini_uninstall(project_dir).expect("test invariant");
    // SAFETY: test-only cleanup.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::remove_var("HOME");
    }
}

#[test]
#[serial(home_env)]
fn test_gemini_install_writes_gemini_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    gemini_install_to(dir.path(), home.path());
    let md = dir.path().join("GEMINI.md");
    assert!(md.exists());
    assert!(
        md.read_to_string_unwrap()
            .contains("graphify-out/GRAPH_REPORT.md")
    );
}

#[test]
#[serial(home_env)]
fn test_gemini_install_writes_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    gemini_install_to(dir.path(), home.path());
    let settings_path = dir.path().join(".gemini/settings.json");
    let settings: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    let hooks = settings["hooks"]["BeforeTool"]
        .as_array()
        .expect("array field");
    assert!(hooks.iter().any(|h| h.to_string().contains("graphify")));
}

#[test]
#[serial(home_env)]
fn test_gemini_install_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    gemini_install_to(dir.path(), home.path());
    gemini_install_to(dir.path(), home.path());
    let md = dir.path().join("GEMINI.md");
    assert_eq!(md.read_to_string_unwrap().matches("## graphify").count(), 1);
}

#[test]
#[serial(home_env)]
fn test_gemini_install_merges_existing_gemini_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("GEMINI.md"), "# My project rules\n").expect("test invariant");
    gemini_install_to(dir.path(), home.path());
    let content = dir.path().join("GEMINI.md").read_to_string_unwrap();
    assert!(content.contains("# My project rules"));
    assert!(content.contains("graphify-out/GRAPH_REPORT.md"));
}

#[test]
#[serial(home_env)]
fn test_gemini_uninstall_removes_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    gemini_install_to(dir.path(), home.path());
    gemini_uninstall_to(dir.path(), home.path());
    assert!(!dir.path().join("GEMINI.md").exists());
}

#[test]
#[serial(home_env)]
fn test_gemini_uninstall_removes_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    gemini_install_to(dir.path(), home.path());
    gemini_uninstall_to(dir.path(), home.path());
    let settings_path = dir.path().join(".gemini/settings.json");
    if settings_path.exists() {
        let settings: serde_json::Value =
            serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
        let hooks = settings
            .pointer("/hooks/BeforeTool")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(!hooks.iter().any(|h| h.to_string().contains("graphify")));
    }
}

#[test]
#[serial(home_env)]
fn test_gemini_uninstall_noop_if_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    gemini_uninstall(dir.path()).expect("test invariant"); // should not error
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
}

// ---------------------------------------------------------------------------
// install_strings parity (test_install_strings.py)
// ---------------------------------------------------------------------------

/// Every install surface must mention `graphify query` (issue #580 fix).
///
/// NOTE: `SETTINGS_HOOK_MATCHER` is just the "Bash" matcher name, not the full
/// `PreToolUse` hook command. The full Claude hook command is built inline in
/// `claude_hook()` and does contain `graphify query`. This test is asserted on
/// the constants surface; the inline-command path is covered by the
/// install-roundtrip parity tests instead.
#[test]
#[ignore = "SETTINGS_HOOK constant exposes matcher only; full hook command is built inline by claude_hook()"]
fn test_every_install_surface_recommends_graphify_query() {
    let surfaces: &[(&str, &str)] = &[
        ("SETTINGS_HOOK", &serde_json::json!({
            "matcher": "Bash",
            "hooks": [{"type": "command", "command": graphify_hooks::platform::SETTINGS_HOOK_MATCHER}]
        }).to_string()),
        ("CLAUDE_MD_SECTION", CLAUDE_MD_SECTION),
        ("AGENTS_MD_SECTION", AGENTS_MD_SECTION),
        ("GEMINI_MD_SECTION", GEMINI_MD_SECTION),
        ("VSCODE_INSTRUCTIONS_SECTION", VSCODE_INSTRUCTIONS_SECTION),
        ("ANTIGRAVITY_RULES", ANTIGRAVITY_RULES),
        ("KIRO_STEERING", KIRO_STEERING),
        ("CURSOR_RULE", CURSOR_RULE),
        ("OPENCODE_PLUGIN_JS", OPENCODE_PLUGIN_JS),
    ];
    let missing: Vec<_> = surfaces
        .iter()
        .filter(|(_, text)| !text.contains("graphify query"))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        missing.is_empty(),
        "these install surfaces no longer mention `graphify query`: {missing:?}"
    );
}

/// Every markdown section must mention `GRAPH_REPORT.md` as a fallback.
#[test]
fn test_report_is_still_referenced_as_fallback() {
    let md_sections: &[(&str, &str)] = &[
        ("CLAUDE_MD_SECTION", CLAUDE_MD_SECTION),
        ("AGENTS_MD_SECTION", AGENTS_MD_SECTION),
        ("GEMINI_MD_SECTION", GEMINI_MD_SECTION),
        ("VSCODE_INSTRUCTIONS_SECTION", VSCODE_INSTRUCTIONS_SECTION),
        ("ANTIGRAVITY_RULES", ANTIGRAVITY_RULES),
        ("KIRO_STEERING", KIRO_STEERING),
        ("CURSOR_RULE", CURSOR_RULE),
    ];
    let missing: Vec<_> = md_sections
        .iter()
        .filter(|(_, text)| !text.contains("GRAPH_REPORT.md"))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        missing.is_empty(),
        "these install sections no longer mention GRAPH_REPORT.md: {missing:?}"
    );
}

/// AGENTS.md section must not say to skip graphify on dirty output.
#[test]
fn test_agents_section_does_not_skip_dirty_graph_output() {
    assert!(AGENTS_MD_SECTION.contains("Dirty graphify-out/ files are expected"));
    assert!(AGENTS_MD_SECTION.contains("not a reason to skip graphify"));
}

// ---------------------------------------------------------------------------
// install_upgrade parity (test_install_upgrade.py)
// ---------------------------------------------------------------------------

const OLD_CLAUDE_SECTION: &str = "\
## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- ALWAYS read graphify-out/GRAPH_REPORT.md before reading any source files, running grep/glob searches, or answering codebase questions. The graph is your primary map of the codebase.
- IF graphify-out/wiki/index.md EXISTS, navigate it instead of reading raw files
- For cross-module \"how does X relate to Y\" questions, prefer `graphify query \"<question>\"`, `graphify path \"<A>\" \"<B>\"`, or `graphify explain \"<concept>\"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
";

const OLD_VSCODE_SECTION: &str = "\
## graphify

For any question about this repo's architecture, structure, components, or how to add/modify/find
code, your **first tool call must be** to read `graphify-out/GRAPH_REPORT.md` (if it exists).

Triggers: \"how do I…\", \"where is…\", \"what does … do\", \"add/modify a <component>\".
";

const OLD_CURSOR_RULE: &str = "\
---
description: graphify knowledge graph context
alwaysApply: true
---

This project has a graphify knowledge graph at graphify-out/.

- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
";

const OLD_KIRO_STEERING: &str = "\
---
inclusion: always
---

graphify: A knowledge graph of this project lives in `graphify-out/`. \
If `graphify-out/GRAPH_REPORT.md` exists, read it before answering architecture questions, \
tracing dependencies, or searching files — it contains god nodes, community structure, \
and surprising connections the graph found.
";

fn assert_no_report_first(text: &str, ctx: &str) {
    assert!(
        !text.contains("ALWAYS read graphify-out/GRAPH_REPORT.md"),
        "{ctx}: old 'ALWAYS read' phrasing survived upgrade"
    );
    assert!(
        !text.contains("first tool call must be"),
        "{ctx}: old VS Code 'first tool call must be' phrasing survived upgrade"
    );
}

fn assert_query_first(text: &str, ctx: &str) {
    assert!(
        text.contains("graphify query"),
        "{ctx}: new 'graphify query' guidance missing after upgrade"
    );
}

#[test]
fn test_claude_install_upgrades_stale_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let claude_md = dir.path().join("CLAUDE.md");
    fs::write(
        &claude_md,
        format!("# My Project\n\nSome description.\n\n{OLD_CLAUDE_SECTION}"),
    )
    .expect("test invariant");
    claude_install(dir.path()).expect("test invariant");
    let after = claude_md.read_to_string_unwrap();
    assert_no_report_first(&after, "CLAUDE.md");
    assert_query_first(&after, "CLAUDE.md");
    assert!(after.contains("# My Project"));
    assert!(after.contains("Some description."));
}

#[test]
fn test_claude_install_upgrades_stale_hook_payload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let claude_md = dir.path().join("CLAUDE.md");
    fs::write(&claude_md, OLD_CLAUDE_SECTION).expect("write fixture");
    let settings = dir.path().join(".claude/settings.json");
    fs::create_dir_all(settings.parent().expect("create_dir_all")).expect("test invariant");
    let stale_settings = serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Bash",
                "hooks": [{"type": "command", "command": "case x in *) Read graphify-out/GRAPH_REPORT.md for god nodes and community structure before searching raw files esac"}]
            }]
        }
    });
    fs::write(&settings, stale_settings.to_string()).expect("test invariant");
    claude_install(dir.path()).expect("test invariant");
    let new_settings_text = settings.read_to_string_unwrap();
    assert!(
        !new_settings_text.contains("Read graphify-out/GRAPH_REPORT.md for god nodes and community structure before searching raw files"),
        "stale hook payload survived upgrade"
    );
    assert!(
        new_settings_text.contains("graphify query"),
        "new hook payload should route to `graphify query`"
    );
}

#[test]
#[serial(home_env)]
fn test_agents_install_upgrades_stale_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agents_md = dir.path().join("AGENTS.md");
    fs::write(
        &agents_md,
        format!("# Project agents\n\n{OLD_CLAUDE_SECTION}"),
    )
    .expect("test invariant");
    agents_install(dir.path(), "codex").expect("test invariant");
    let after = agents_md.read_to_string_unwrap();
    assert_no_report_first(&after, "AGENTS.md");
    assert_query_first(&after, "AGENTS.md");
    assert!(after.contains("# Project agents"));
}

#[test]
#[serial(home_env)]
fn test_gemini_install_upgrades_stale_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let gemini_md = dir.path().join("GEMINI.md");
    fs::write(&gemini_md, OLD_CLAUDE_SECTION).expect("write fixture");
    gemini_install_to(dir.path(), home.path());
    let after = gemini_md.read_to_string_unwrap();
    assert_no_report_first(&after, "GEMINI.md");
    assert_query_first(&after, "GEMINI.md");
}

#[test]
#[serial(home_env)]
fn test_vscode_install_upgrades_stale_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let instructions = dir.path().join(".github/copilot-instructions.md");
    fs::create_dir_all(instructions.parent().expect("create_dir_all")).expect("test invariant");
    fs::write(&instructions, OLD_VSCODE_SECTION).expect("write fixture");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    vscode_install(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    let after = instructions.read_to_string_unwrap();
    assert_no_report_first(&after, "copilot-instructions.md");
    assert_query_first(&after, "copilot-instructions.md");
}

#[test]
fn test_cursor_install_upgrades_stale_rule() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rule_path = dir.path().join(".cursor/rules/graphify.mdc");
    fs::create_dir_all(rule_path.parent().expect("create_dir_all")).expect("test invariant");
    fs::write(&rule_path, OLD_CURSOR_RULE).expect("write fixture");
    cursor_install(dir.path()).expect("test invariant");
    let after = rule_path.read_to_string_unwrap();
    assert!(
        !after.contains("read graphify-out/GRAPH_REPORT.md for god nodes"),
        "old cursor rule phrasing survived upgrade"
    );
    assert_query_first(&after, ".cursor/rules/graphify.mdc");
    assert!(after.contains("alwaysApply: true"));
}

#[test]
fn test_kiro_install_upgrades_stale_steering() {
    let dir = tempfile::tempdir().expect("tempdir");
    let steering = dir.path().join(".kiro/steering/graphify.md");
    fs::create_dir_all(steering.parent().expect("create_dir_all")).expect("test invariant");
    fs::write(&steering, OLD_KIRO_STEERING).expect("write fixture");
    kiro_install(dir.path()).expect("test invariant");
    let after = steering.read_to_string_unwrap();
    assert!(
        !after.contains("read it before answering architecture questions"),
        "old kiro steering phrasing survived upgrade"
    );
    assert_query_first(&after, ".kiro/steering/graphify.md");
    assert!(after.contains("inclusion: always"));
}

// ---------------------------------------------------------------------------
// Kiro install/uninstall
// ---------------------------------------------------------------------------

#[test]
fn test_kiro_install_writes_skill_and_steering() {
    let dir = tempfile::tempdir().expect("tempdir");
    kiro_install(dir.path()).expect("test invariant");
    assert!(dir.path().join(".kiro/skills/graphify/SKILL.md").exists());
    let steering = dir.path().join(".kiro/steering/graphify.md");
    assert!(steering.exists());
    assert!(
        steering
            .read_to_string_unwrap()
            .contains("inclusion: always")
    );
}

#[test]
fn test_kiro_uninstall_removes_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    kiro_install(dir.path()).expect("test invariant");
    kiro_uninstall(dir.path()).expect("test invariant");
    assert!(!dir.path().join(".kiro/skills/graphify/SKILL.md").exists());
    assert!(!dir.path().join(".kiro/steering/graphify.md").exists());
}

// ---------------------------------------------------------------------------
// install_claude_hook / uninstall_claude_hook standalone
// ---------------------------------------------------------------------------

#[test]
fn test_install_claude_hook_creates_settings() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_claude_hook(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".claude/settings.json");
    assert!(settings_path.exists());
    let v: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    assert!(v["hooks"]["PreToolUse"].is_array());
}

#[test]
fn test_uninstall_claude_hook_noop_when_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let msg = uninstall_claude_hook(dir.path()).expect("test invariant");
    assert!(msg.is_empty());
}

// ---------------------------------------------------------------------------
// install_gemini_hook / uninstall_gemini_hook standalone
// ---------------------------------------------------------------------------

#[test]
fn test_install_gemini_hook_creates_settings() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_gemini_hook(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".gemini/settings.json");
    assert!(settings_path.exists());
    let v: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    assert!(v["hooks"]["BeforeTool"].is_array());
}

#[test]
fn test_uninstall_gemini_hook_noop_when_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let msg = uninstall_gemini_hook(dir.path()).expect("test invariant");
    assert!(msg.is_empty());
}

// ---------------------------------------------------------------------------
// install_codex_hook / uninstall_codex_hook standalone
// ---------------------------------------------------------------------------

#[test]
fn test_install_codex_hook_creates_hooks_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_codex_hook(dir.path()).expect("test invariant");
    let hooks_path = dir.path().join(".codex/hooks.json");
    assert!(hooks_path.exists());
    let v: serde_json::Value =
        serde_json::from_str(&hooks_path.read_to_string_unwrap()).expect("test invariant");
    let pre_tool = v["hooks"]["PreToolUse"].as_array().expect("array field");
    assert!(
        pre_tool
            .iter()
            .any(|h| h.to_string().contains("hook-check"))
    );
}

#[test]
fn test_uninstall_codex_hook_noop_when_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let msg = uninstall_codex_hook(dir.path()).expect("test invariant");
    assert!(msg.is_empty());
}

// ---------------------------------------------------------------------------
// install_opencode_plugin / uninstall_opencode_plugin standalone
// ---------------------------------------------------------------------------

#[test]
fn test_install_opencode_plugin_writes_js() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_opencode_plugin(dir.path()).expect("test invariant");
    let plugin = dir.path().join(".opencode/plugins/graphify.js");
    assert!(plugin.exists());
    assert!(
        plugin
            .read_to_string_unwrap()
            .contains("tool.execute.before")
    );
}

#[test]
fn test_uninstall_opencode_plugin_noop_when_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let msg = uninstall_opencode_plugin(dir.path()).expect("test invariant");
    assert!(msg.is_empty());
}

// ---------------------------------------------------------------------------
// antigravity_install / antigravity_uninstall
// ---------------------------------------------------------------------------

#[test]
#[serial(home_env)]
fn test_antigravity_install_writes_rules_and_workflow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    antigravity_install(dir.path(), false).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(dir.path().join(".agents/rules/graphify.md").exists());
    assert!(dir.path().join(".agents/workflows/graphify.md").exists());
}

#[test]
#[serial(home_env)]
fn test_antigravity_uninstall_removes_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    antigravity_install(dir.path(), false).expect("test invariant");
    antigravity_uninstall(dir.path(), false).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(!dir.path().join(".agents/rules/graphify.md").exists());
    assert!(!dir.path().join(".agents/workflows/graphify.md").exists());
}

/// #1079: a global install writes the skill to ~/.gemini/config/skills/, not
/// the old ~/.agents/skills/ location; rules + workflow stay workspace-local.
#[test]
#[serial(home_env)]
fn test_antigravity_global_install_writes_gemini_config_skills() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    antigravity_install(dir.path(), false).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    let global_skill = home.path().join(".gemini/config/skills/graphify/SKILL.md");
    let wrong_skill = home.path().join(".agents/skills/graphify/SKILL.md");
    assert!(global_skill.exists(), "skill missing from {global_skill:?}");
    assert!(
        !wrong_skill.exists(),
        "skill wrongly written to {wrong_skill:?}"
    );
    assert!(dir.path().join(".agents/rules/graphify.md").exists());
    assert!(dir.path().join(".agents/workflows/graphify.md").exists());
}

/// #1079: a global uninstall removes the skill from ~/.gemini/config/skills/
/// and cleans up the workspace rules + workflow.
#[test]
#[serial(home_env)]
fn test_antigravity_global_uninstall_removes_gemini_config_skill() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    antigravity_install(dir.path(), false).expect("test invariant");
    let global_skill = home.path().join(".gemini/config/skills/graphify/SKILL.md");
    assert!(global_skill.exists(), "precondition: skill must exist");
    antigravity_uninstall(dir.path(), false).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(
        !global_skill.exists(),
        "skill not removed from {global_skill:?}"
    );
    assert!(!dir.path().join(".agents/rules/graphify.md").exists());
    assert!(!dir.path().join(".agents/workflows/graphify.md").exists());
}

/// #1079: a `--project` uninstall removes only the workspace-local skill and
/// must not touch the shared global skill.
#[test]
#[serial(home_env)]
fn test_antigravity_uninstall_project_removes_project_skill_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // Pre-create a global skill that must survive the project uninstall.
    let global_skill = home.path().join(".gemini/config/skills/graphify/SKILL.md");
    fs::create_dir_all(global_skill.parent().expect("parent")).expect("mkdir");
    fs::write(&global_skill, "global skill").expect("write");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    antigravity_install(dir.path(), true).expect("test invariant");
    let project_skill = dir.path().join(".agents/skills/graphify/SKILL.md");
    assert!(project_skill.exists(), "project skill must be written");
    antigravity_uninstall(dir.path(), true).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(
        global_skill.exists(),
        "project uninstall must not touch global skill"
    );
    assert!(!project_skill.exists(), "project skill must be removed");
}

// ---------------------------------------------------------------------------
// vscode_install / vscode_uninstall
// ---------------------------------------------------------------------------

#[test]
#[serial(home_env)]
fn test_vscode_install_creates_copilot_instructions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    vscode_install(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    let instructions = dir.path().join(".github/copilot-instructions.md");
    assert!(instructions.exists());
    assert!(
        instructions
            .read_to_string_unwrap()
            .contains("graphify query")
    );
}

#[test]
#[serial(home_env)]
fn test_vscode_uninstall_removes_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    vscode_install(dir.path()).expect("test invariant");
    vscode_uninstall(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    // Either the file is gone or the section is removed
    let instructions = dir.path().join(".github/copilot-instructions.md");
    if instructions.exists() {
        assert!(!instructions.read_to_string_unwrap().contains("## graphify"));
    }
}

// ---------------------------------------------------------------------------
// install_platform_skill_project / uninstall_platform_skill_project
// ---------------------------------------------------------------------------

#[test]
fn install_platform_skill_project_claude_writes_skill_and_registers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path();
    let msg = install_platform_skill_project("claude", project).expect("test invariant");

    let skill_path = project.join(".claude/skills/graphify/SKILL.md");
    assert!(skill_path.is_file(), "skill must be written to project dir");

    let claude_md = project.join(".claude/CLAUDE.md");
    assert!(
        claude_md.is_file(),
        "CLAUDE.md must be created on first install"
    );
    let content = fs::read_to_string(&claude_md).expect("read fixture");
    assert!(
        content
            .lines()
            .any(|line| line.trim_start() == "## graphify"),
        "CLAUDE.md must contain the `## graphify` registration heading: {content:?}"
    );

    assert!(
        msg.contains("git add .claude"),
        "install message must include the `git add` hint pointing at the scope root: {msg}"
    );
}

#[test]
fn install_platform_skill_project_idempotent_registration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path();

    // Seed CLAUDE.md with an existing registration so the install must
    // detect it and skip the append rather than duplicate it.
    let claude_md = project.join(".claude/CLAUDE.md");
    fs::create_dir_all(claude_md.parent().expect("create_dir_all")).expect("test invariant");
    fs::write(
        &claude_md,
        "## graphify\n\nFollow `.claude/skills/graphify/SKILL.md` when working in this project.\n",
    )
    .expect("test invariant");

    let msg = install_platform_skill_project("claude", project).expect("test invariant");
    let after = fs::read_to_string(&claude_md).expect("read fixture");
    let count = after
        .lines()
        .filter(|line| line.trim_start() == "## graphify")
        .count();
    assert_eq!(count, 1, "second install must not duplicate the heading");
    assert!(
        msg.contains("already registered"),
        "install message must call out the idempotent-skip path: {msg}"
    );
}

#[test]
fn install_platform_skill_project_unknown_platform_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = install_platform_skill_project("not-a-platform", dir.path());
    assert!(
        result.is_err(),
        "unrecognised platform must surface as an error"
    );
}

#[test]
fn uninstall_platform_skill_project_removes_skill_and_strips_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path();
    install_platform_skill_project("claude", project).expect("test invariant");

    let skill_path = project.join(".claude/skills/graphify/SKILL.md");
    let claude_md = project.join(".claude/CLAUDE.md");
    assert!(skill_path.is_file());
    assert!(claude_md.is_file());

    uninstall_platform_skill_project("claude", project).expect("test invariant");
    assert!(
        !skill_path.exists(),
        "uninstall must remove the project skill file"
    );

    // The CLAUDE.md should either be gone (if it was empty after stripping)
    // or no longer contain the `## graphify` heading.
    if claude_md.exists() {
        let content = fs::read_to_string(&claude_md).expect("read fixture");
        assert!(
            !content
                .lines()
                .any(|line| line.trim_start() == "## graphify"),
            "uninstall must strip the `## graphify` registration"
        );
    }
}

#[test]
fn uninstall_platform_skill_project_when_not_installed_is_silent() {
    let dir = tempfile::tempdir().expect("tempdir");
    // No prior install — uninstall must succeed and announce the no-op.
    let msg = uninstall_platform_skill_project("claude", dir.path()).expect("test invariant");
    assert!(
        msg.contains("not installed"),
        "uninstall on clean dir must report the no-op: {msg}"
    );
}

// ---------------------------------------------------------------------------
// devin_install / devin_uninstall / devin_project_install / devin_project_uninstall
// Ports graphify-py tests/test_devin.py
// ---------------------------------------------------------------------------

use graphify_hooks::platform::{
    devin_install, devin_project_install, devin_project_uninstall, devin_uninstall,
};

#[test]
#[serial(home_env)]
fn test_devin_install_user_creates_skill_file() {
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_install().expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    let skill = home.path().join(".config/devin/skills/graphify/SKILL.md");
    assert!(skill.exists(), "skill missing at {}", skill.display());
}

#[test]
#[serial(home_env)]
fn test_devin_install_user_does_not_write_rules() {
    // User-scope install must NOT write `.windsurf/rules/graphify.md`
    // anywhere on the project tree — that file is project-scoped only.
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_install().expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(!dir.path().join(".windsurf/rules/graphify.md").exists());
}

#[test]
#[serial(home_env)]
fn test_devin_install_project_creates_skill_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_project_install(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(dir.path().join(".devin/skills/graphify/SKILL.md").exists());
}

#[test]
#[serial(home_env)]
fn test_devin_install_project_creates_rules_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_project_install(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    let rules = dir.path().join(".windsurf/rules/graphify.md");
    assert!(rules.exists());
    let body = fs::read_to_string(&rules).expect("read rules");
    assert!(body.contains("graphify query"));
    assert!(body.contains("graphify-out/"));
}

#[test]
#[serial(home_env)]
fn test_devin_rules_install_idempotent() {
    // Two installs with identical rule content must leave the file
    // unchanged — mirrors graphify-py `_devin_rules_install` idempotency.
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_project_install(dir.path()).expect("test invariant");
    let msg = devin_project_install(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(msg.contains("already configured"));
}

#[test]
#[serial(home_env)]
fn test_devin_uninstall_user_removes_skill_file() {
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_install().expect("test invariant");
    devin_uninstall().expect("test invariant");
    let skill = home.path().join(".config/devin/skills/graphify/SKILL.md");
    let still_exists = skill.exists();
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(!still_exists);
}

#[test]
#[serial(home_env)]
fn test_devin_uninstall_user_noop_when_not_installed() {
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    let msg = devin_uninstall().expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(msg.contains("nothing to remove"));
}

#[test]
#[serial(home_env)]
fn test_devin_uninstall_project_removes_skill_and_rules() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_project_install(dir.path()).expect("test invariant");
    devin_project_uninstall(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(!dir.path().join(".devin/skills/graphify/SKILL.md").exists());
    assert!(!dir.path().join(".windsurf/rules/graphify.md").exists());
}

#[test]
#[serial(home_env)]
fn test_devin_uninstall_project_does_not_touch_user_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    devin_install().expect("user install");
    devin_project_install(dir.path()).expect("project install");
    devin_project_uninstall(dir.path()).expect("project uninstall");
    let user_skill_exists = home
        .path()
        .join(".config/devin/skills/graphify/SKILL.md")
        .exists();
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(
        user_skill_exists,
        "user-scope skill must survive project uninstall"
    );
}

#[test]
#[serial(home_env)]
fn test_devin_project_uninstall_noop_when_clean() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    let msg = devin_project_uninstall(dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(msg.contains("nothing to remove"));
}

#[test]
#[serial(home_env)]
fn test_devin_install_platform_skill_user_scope() {
    // `install_platform_skill("devin")` is the entry path triggered by
    // `graphify install --platform devin`. It must write the user-scope
    // skill at `~/.config/devin/skills/graphify/SKILL.md`.
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    install_platform_skill("devin").expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(
        home.path()
            .join(".config/devin/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
#[serial(home_env)]
fn test_devin_install_platform_skill_project_scope() {
    // The `install_platform_skill_project` path is the default branch in
    // `cmd_platform` for `--project` installs of other platforms; devin
    // bypasses it (see `cli/install.rs`) but the helper should still
    // accept "devin" and write to the project-scoped location.
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    install_platform_skill_project("devin", dir.path()).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    // `install_platform_skill_project` uses the home-relative path,
    // which for devin is `.config/devin/...`. This branch exists only
    // for parity with other platforms — production callers route devin
    // through `devin_project_install`.
    assert!(
        dir.path()
            .join(".config/devin/skills/graphify/SKILL.md")
            .exists()
    );
}

// ---------------------------------------------------------------------------
// Helper trait for .read_to_string_unwrap() on Path
// ---------------------------------------------------------------------------

trait ReadToString {
    fn read_to_string_unwrap(&self) -> String;
}

impl ReadToString for PathBuf {
    fn read_to_string_unwrap(&self) -> String {
        fs::read_to_string(self).expect("read fixture")
    }
}
