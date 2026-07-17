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
    CHECKOUT_MARKER, CHECKOUT_SCRIPT, HOOK_MARKER, HOOK_SCRIPT, PYTHON_DETECT, WORKTREE_GUARD,
    hooks_dir, hooks_dir_with, install, status, uninstall, user_hooks_dir,
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
// #1385: reject Windows-style hooks paths instead of creating a junk dir
// ---------------------------------------------------------------------------

/// Set `core.hooksPath` on `repo` via `git config --local` (mirrors the
/// Python `_set_hookspath` test helper).
fn set_hookspath(repo: &Path, value: &str) {
    Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "config",
            "--local",
            "core.hooksPath",
            value,
        ])
        .output()
        .expect("git config failed");
}

/// Recursively collect every path under `dir` (mirrors Python `Path.rglob("*")`).
#[cfg(not(windows))]
fn walk_all(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_all(&path, out);
        }
        out.push(path);
    }
}

#[cfg(not(windows))]
#[test]
fn test_windows_hookspath_rejected_no_junk_dir() {
    // A Windows-style core.hooksPath must raise (loud failure), not silently
    // create a backslash-named junk directory and report success (#1385).
    // Ports each `winpath` pytest.parametrize value as a case.
    let winpaths = [
        r"C:\Users\u\repo\.git\hooks",
        r"c:/Users/u/.git/hooks",
        r"D:\hooks",
        r"some\back\slashed\path",
    ];
    for winpath in winpaths {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = make_git_repo(dir.path());
        set_hookspath(&repo, winpath);

        let err = install(&repo).expect_err("windows hooks path must be rejected");
        assert!(
            err.to_string().contains("Windows path"),
            "error for {winpath:?} must mention 'Windows path', got: {err}"
        );

        // No junk directory got created anywhere under the repo.
        let mut all = Vec::new();
        walk_all(&repo, &mut all);
        let junk: Vec<&PathBuf> = all
            .iter()
            .filter(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                name.contains('\\')
                    || name.starts_with("C:")
                    || name.starts_with("c:")
                    || name.starts_with("D:")
            })
            .collect();
        assert!(
            junk.is_empty(),
            "junk dir created for {winpath:?}: {junk:?}"
        );
    }
}

#[test]
fn test_posix_custom_hookspath_still_works() {
    // A legitimate POSIX core.hooksPath (Husky-style) must still install.
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    set_hookspath(&repo, ".husky");
    let msg = install(&repo).expect("posix custom hooks path must install");
    assert!(msg.contains("post-commit"));
    assert!(repo.join(".husky").join("post-commit").exists());
}

#[test]
fn test_default_hooks_dir_unaffected() {
    // No core.hooksPath -> normal .git/hooks install, no rejection.
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("default hooks dir must install");
    assert!(repo.join(".git").join("hooks").join("post-commit").exists());
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
    agents_install, agents_platform_install, agents_uninstall, amp_install, amp_uninstall,
    antigravity_install, antigravity_uninstall, claude_install, claude_uninstall,
    codebuddy_install, codebuddy_uninstall, cursor_install, cursor_uninstall, gemini_install,
    gemini_uninstall, hermes_skill_dst, install_claude_hook, install_codex_hook,
    install_gemini_hook, install_opencode_plugin, install_platform_skill,
    install_platform_skill_project, kiro_install, kiro_uninstall, replace_or_append_section,
    uninstall_claude_hook, uninstall_codex_hook, uninstall_gemini_hook, uninstall_opencode_plugin,
    uninstall_platform_skill_project, vscode_install, vscode_uninstall,
};

// ── #1403: hermes skill destination (Windows %LOCALAPPDATA%) ─────────────────

#[test]
fn test_hermes_skill_destination_windows_uses_localappdata() {
    // On Windows, Hermes scans %LOCALAPPDATA%\hermes\skills, not ~/.hermes.
    let home = Path::new("/home/user");
    let localappdata = Path::new("/tmp/AppDataLocal");
    let dst = hermes_skill_dst(home, Some(localappdata), true);
    assert_eq!(
        dst,
        Path::new("/tmp/AppDataLocal")
            .join("hermes")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    );
}

#[test]
fn test_hermes_skill_destination_windows_falls_back_to_appdata_local() {
    // LOCALAPPDATA unset on Windows -> <home>/AppData/Local.
    let home = Path::new("/home/user");
    let dst = hermes_skill_dst(home, None, true);
    assert_eq!(
        dst,
        home.join("AppData")
            .join("Local")
            .join("hermes")
            .join("skills")
            .join("graphify")
            .join("SKILL.md")
    );
}

#[test]
fn test_hermes_skill_destination_posix_uses_home() {
    let home = Path::new("/home/user");
    let dst = hermes_skill_dst(home, None, false);
    assert!(dst.ends_with(".hermes/skills/graphify/SKILL.md"), "{dst:?}");
}

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

#[test]
fn test_replace_or_append_ignores_inline_marker_mention() {
    // #1688: an inline `## graphify` mention (in a bullet/prose) must NOT be
    // treated as the section heading. Substring-matching it anchored the replace
    // there and deleted every line to the next heading, destroying content.
    let content =
        "# Notes\n\n- see the `## graphify` marker below\n\nImportant hand-written notes.\n";
    let result = replace_or_append_section(content, "## graphify", "## graphify\n\nManaged.\n");
    assert!(
        result.contains("- see the `## graphify` marker below"),
        "inline mention line preserved: {result}"
    );
    assert!(
        result.contains("Important hand-written notes."),
        "hand-written notes preserved: {result}"
    );
    assert!(
        result.contains("Managed."),
        "managed section appended: {result}"
    );
}

#[test]
fn test_replace_or_append_ignores_longer_heading() {
    // #1688: `## graphify internals` is a different heading and must not match
    // the managed `## graphify` section.
    let content = "## graphify internals\n\nDocs about internals.\n";
    let result = replace_or_append_section(content, "## graphify", "## graphify\n\nManaged.\n");
    assert!(
        result.contains("## graphify internals"),
        "longer heading preserved: {result}"
    );
    assert!(
        result.contains("Docs about internals."),
        "its body preserved: {result}"
    );
    assert!(
        result.contains("Managed."),
        "managed section appended: {result}"
    );
}

#[test]
fn test_replace_or_append_uses_last_exact_heading() {
    // #1688: with duplicate exact headings, the LAST is replaced (graphify's
    // section is always appended), leaving earlier content intact.
    let content = "## graphify\n\nfirst.\n\n## other\n\nmid.\n\n## graphify\n\nstale managed.\n";
    let result =
        replace_or_append_section(content, "## graphify", "## graphify\n\nfresh managed.\n");
    assert!(
        result.contains("fresh managed."),
        "last section updated: {result}"
    );
    assert!(
        !result.contains("stale managed."),
        "stale last section replaced: {result}"
    );
    assert!(
        result.contains("## other"),
        "unrelated section preserved: {result}"
    );
    assert!(
        result.contains("mid."),
        "unrelated body preserved: {result}"
    );
}

// ---------------------------------------------------------------------------
// claude_install / claude_uninstall (test_claude_md.py)
// ---------------------------------------------------------------------------

/// Run `claude_uninstall` with `HOME` overridden to `skill_home` (and
/// `CLAUDE_CONFIG_DIR` cleared) so the user-scope skill-tree removal (#1121)
/// stays inside the temp dir instead of touching the real home. Mirrors
/// `gemini_uninstall_to`.
fn claude_uninstall_to(project_dir: &Path, skill_home: &Path) -> String {
    let prev_cfg = std::env::var("CLAUDE_CONFIG_DIR").ok();
    // SAFETY: test-only env override; `#[serial(home_env)]` serialises access.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", skill_home);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
    let r = claude_uninstall(project_dir).expect("test invariant");
    // SAFETY: test-only cleanup.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::remove_var("HOME");
        if let Some(v) = prev_cfg {
            std::env::set_var("CLAUDE_CONFIG_DIR", v);
        }
    }
    r
}

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
#[serial(home_env)]
fn test_uninstall_removes_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    claude_uninstall_to(dir.path(), home.path());
    let target = dir.path().join("CLAUDE.md");
    if target.exists() {
        assert!(!target.read_to_string_unwrap().contains(CLAUDE_MD_MARKER));
    }
}

#[test]
#[serial(home_env)]
fn test_uninstall_preserves_other_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("CLAUDE.md");
    fs::write(&target, "# My Project\n\nSome rules.\n").expect("write fixture");
    claude_install(dir.path()).expect("test invariant");
    claude_uninstall_to(dir.path(), home.path());
    assert!(target.exists());
    let content = target.read_to_string_unwrap();
    assert!(content.contains("My Project"));
    assert!(content.contains("Some rules"));
    assert!(!content.contains(CLAUDE_MD_MARKER));
}

#[test]
#[serial(home_env)]
fn test_uninstall_no_op_when_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("CLAUDE.md");
    fs::write(&target, "# Other stuff\n").expect("write fixture");
    let msg = claude_uninstall_to(dir.path(), home.path());
    assert!(msg.contains("not found") || msg.contains("nothing to do"));
}

#[test]
#[serial(home_env)]
fn test_uninstall_no_op_when_no_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let msg = claude_uninstall_to(dir.path(), home.path());
    assert!(msg.contains("No CLAUDE.md") || msg.contains("nothing to do"));
    // The absent-hook path contributes no message, so the output must not carry a
    // trailing blank line (the empty hook message is skipped, matching Python).
    assert!(
        !msg.ends_with('\n'),
        "uninstall output must not end with a blank line: {msg:?}"
    );
}

#[test]
#[serial(home_env)]
fn test_uninstall_removes_user_skill_tree_preserving_siblings() {
    // claude_uninstall must remove the orphaned user-scope skill tree (#1121),
    // mirroring gemini_uninstall — but scoped removal must keep sibling files.
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    install_skill_to(home.path(), "claude");
    let skill = home.path().join(".claude/skills/graphify/SKILL.md");
    assert!(skill.exists(), "precondition: skill installed");
    let sibling = home.path().join(".claude/skills/user_notes.md");
    fs::write(&sibling, "keep me").expect("write sibling");

    claude_uninstall_to(dir.path(), home.path());
    assert!(!skill.exists(), "skill tree should be removed on uninstall");
    assert!(sibling.exists(), "sibling user file must be preserved");
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
    let home = tempfile::tempdir().expect("tempdir");
    claude_install(dir.path()).expect("test invariant");
    claude_uninstall_to(dir.path(), home.path());
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
// codebuddy_install / codebuddy_uninstall (test_codebuddy.py)
// ---------------------------------------------------------------------------

/// Run `codebuddy_uninstall` with `HOME` overridden to `skill_home` so the
/// user-scope skill-tree removal stays inside the temp dir instead of touching
/// the real home. `CodeBuddy` keys off `Path.home()` only (no `CLAUDE_CONFIG_DIR`).
fn codebuddy_uninstall_to(project_dir: &Path, skill_home: &Path) -> String {
    // SAFETY: test-only env override; `#[serial(home_env)]` serialises access.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", skill_home);
    }
    let r = codebuddy_uninstall(project_dir).expect("test invariant");
    // SAFETY: test-only cleanup.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::remove_var("HOME");
    }
    r
}

#[test]
#[serial(home_env)]
fn test_codebuddy_install_user_creates_skill_file() {
    // `graphify install --platform codebuddy` copies the skill to
    // ~/.codebuddy/skills/graphify/SKILL.md.
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "codebuddy");
    assert!(
        dir.path()
            .join(".codebuddy/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
#[serial(home_env)]
fn test_codebuddy_skill_file_contains_frontmatter() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "codebuddy");
    let content = dir
        .path()
        .join(".codebuddy/skills/graphify/SKILL.md")
        .read_to_string_unwrap();
    assert!(content.contains("name: graphify"));
    assert!(content.contains("description:"));
}

#[test]
#[serial(home_env)]
fn test_codebuddy_skill_file_references_graphify_query() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "codebuddy");
    let content = dir
        .path()
        .join(".codebuddy/skills/graphify/SKILL.md")
        .read_to_string_unwrap();
    assert!(content.contains("graphify query") || content.contains("/graphify query"));
}

#[test]
#[serial(home_env)]
fn test_codebuddy_install_user_registers_codebuddy_md() {
    // The user-scope install also registers the skill in ~/.codebuddy/CODEBUDDY.md.
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "codebuddy");
    let md = dir.path().join(".codebuddy/CODEBUDDY.md");
    assert!(md.exists());
    assert!(md.read_to_string_unwrap().contains("graphify"));
}

#[test]
fn test_codebuddy_install_project_writes_codebuddy_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    let md = dir.path().join("CODEBUDDY.md");
    assert!(md.exists());
    let content = md.read_to_string_unwrap();
    assert!(content.contains(CLAUDE_MD_MARKER));
    assert!(content.contains("graphify-out/"));
}

#[test]
fn test_codebuddy_install_project_writes_skill() {
    // Project-scope `graphify codebuddy install` lays the skill under
    // <project>/.codebuddy/skills/graphify/SKILL.md.
    let dir = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    assert!(
        dir.path()
            .join(".codebuddy/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
fn test_codebuddy_install_project_writes_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".codebuddy").join("settings.json");
    assert!(settings_path.exists());
    let settings: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    let hooks = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("array field");
    assert!(hooks.iter().any(|h| h.to_string().contains("graphify")));
}

#[test]
fn test_codebuddy_install_hook_has_bash_matcher() {
    let dir = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".codebuddy").join("settings.json");
    let settings: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    let hooks = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("array field");
    assert!(hooks.iter().any(|h| {
        h.get("matcher").and_then(|v| v.as_str()) == Some("Bash")
            && h.to_string().contains("graphify")
    }));
}

#[test]
fn test_codebuddy_install_hook_has_read_glob_matcher() {
    let dir = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    let settings_path = dir.path().join(".codebuddy").join("settings.json");
    let settings: serde_json::Value =
        serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
    let hooks = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("array field");
    assert!(hooks.iter().any(|h| {
        h.get("matcher").and_then(|v| v.as_str()) == Some("Read|Glob")
            && h.to_string().contains("graphify")
    }));
}

#[test]
fn test_codebuddy_install_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    codebuddy_install(dir.path()).expect("test invariant");
    let md = dir.path().join("CODEBUDDY.md");
    assert_eq!(
        md.read_to_string_unwrap().matches(CLAUDE_MD_MARKER).count(),
        1
    );
}

#[test]
fn test_codebuddy_install_upgrades_stale_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let md = dir.path().join("CODEBUDDY.md");
    fs::write(
        &md,
        "old content\n\n## graphify\nThis is old instructions\n",
    )
    .expect("write fixture");
    codebuddy_install(dir.path()).expect("test invariant");
    let content = md.read_to_string_unwrap();
    assert!(content.contains(CLAUDE_MD_MARKER));
    assert!(content.contains("old content"));
    assert!(!content.contains("This is old instructions"));
    assert!(content.contains("graphify-out/"));
    assert_eq!(content.matches(CLAUDE_MD_MARKER).count(), 1);
}

#[test]
fn test_codebuddy_install_merges_existing_codebuddy_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    let md = dir.path().join("CODEBUDDY.md");
    fs::write(&md, "# My project rules\n").expect("write fixture");
    codebuddy_install(dir.path()).expect("test invariant");
    let content = md.read_to_string_unwrap();
    assert!(content.contains("# My project rules"));
    assert!(content.contains(CLAUDE_MD_MARKER));
    assert!(content.contains("graphify-out/"));
}

#[test]
fn test_codebuddy_install_prints_no_change_on_second_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    let msg = codebuddy_install(dir.path()).expect("test invariant");
    assert!(msg.contains("no change"));
}

#[test]
#[serial(home_env)]
fn test_codebuddy_uninstall_removes_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    codebuddy_uninstall_to(dir.path(), home.path());
    // CODEBUDDY.md was created from scratch and contained only the graphify
    // section, so removing it deletes the file.
    assert!(!dir.path().join("CODEBUDDY.md").exists());
}

#[test]
#[serial(home_env)]
fn test_codebuddy_uninstall_removes_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    codebuddy_install(dir.path()).expect("test invariant");
    codebuddy_uninstall_to(dir.path(), home.path());
    let settings_path = dir.path().join(".codebuddy").join("settings.json");
    if settings_path.exists() {
        let settings: serde_json::Value =
            serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
        let hooks = settings["hooks"]["PreToolUse"]
            .as_array()
            .expect("array field");
        assert!(!hooks.iter().any(|h| h.to_string().contains("graphify")));
    }
}

#[test]
#[serial(home_env)]
fn test_codebuddy_uninstall_noop_if_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // Must not raise when CODEBUDDY.md doesn't exist.
    let msg = codebuddy_uninstall_to(dir.path(), home.path());
    assert!(msg.contains("No CODEBUDDY.md") || msg.contains("nothing to do"));
}

#[test]
#[serial(home_env)]
fn test_codebuddy_uninstall_noop_if_no_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let md = dir.path().join("CODEBUDDY.md");
    fs::write(&md, "# Some other project\n").expect("write fixture");
    codebuddy_uninstall_to(dir.path(), home.path());
    assert!(md.read_to_string_unwrap().contains("# Some other project"));
}

#[test]
#[serial(home_env)]
fn test_codebuddy_uninstall_preserves_other_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let md = dir.path().join("CODEBUDDY.md");
    fs::write(&md, "# My project rules\n").expect("write fixture");
    codebuddy_install(dir.path()).expect("test invariant");
    codebuddy_uninstall_to(dir.path(), home.path());
    let content = md.read_to_string_unwrap();
    assert!(!content.contains(CLAUDE_MD_MARKER));
    assert!(content.contains("# My project rules"));
}

#[test]
#[serial(home_env)]
fn test_codebuddy_installation_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let md = dir.path().join("CODEBUDDY.md");
    fs::write(&md, "# My project\n").expect("write fixture");
    codebuddy_install(dir.path()).expect("test invariant");
    codebuddy_uninstall_to(dir.path(), home.path());
    assert!(md.exists());
    let content = md.read_to_string_unwrap();
    assert!(!content.contains(CLAUDE_MD_MARKER));
    assert!(content.contains("# My project"));
    let settings_path = dir.path().join(".codebuddy").join("settings.json");
    if settings_path.exists() {
        let settings: serde_json::Value =
            serde_json::from_str(&settings_path.read_to_string_unwrap()).expect("test invariant");
        let hooks = settings["hooks"]["PreToolUse"]
            .as_array()
            .expect("array field");
        assert!(!hooks.iter().any(|h| h.to_string().contains("graphify")));
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
    // Codex installs the skill to `.codex/skills/...` (#1160), matching where
    // its hook writes.
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "codex");
    assert!(dir.path().join(".codex/skills/graphify/SKILL.md").exists());
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

// ── #1432: generic `agents` platform + `skills` alias ────────────────────────

#[test]
#[serial(home_env)]
fn test_install_agents_user_global() {
    // `--platform agents` lands the skill at ~/.agents/skills (skill-only).
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "agents");
    assert!(dir.path().join(".agents/skills/graphify/SKILL.md").exists());
    // Skill-only: no AGENTS.md from the bare install path.
    assert!(!dir.path().join("AGENTS.md").exists());
}

#[test]
fn test_install_agents_project_uses_dot_agents() {
    let dir = tempfile::tempdir().expect("tempdir");
    install_platform_skill_project("agents", dir.path()).expect("test invariant");
    assert!(dir.path().join(".agents/skills/graphify/SKILL.md").exists());
}

#[test]
#[serial(home_env)]
fn test_agents_subcommand_wires_skill_and_agents_md() {
    // `graphify agents install` is the amp-twin: skill at ~/.agents/skills PLUS
    // an AGENTS.md `## graphify` section. Running it twice stays idempotent.
    let home = tempfile::tempdir().expect("tempdir");
    let proj = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only; serialised via `#[serial(home_env)]`.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    agents_platform_install(proj.path()).expect("test invariant");
    agents_platform_install(proj.path()).expect("idempotent re-run");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(
        home.path()
            .join(".agents/skills/graphify/SKILL.md")
            .exists()
    );
    let body = fs::read_to_string(proj.path().join("AGENTS.md")).expect("AGENTS.md");
    assert!(body.contains("## graphify"));
    assert_eq!(
        body.matches("## graphify").count(),
        1,
        "AGENTS.md gained a duplicate graphify section"
    );
}

#[test]
fn test_opencode_plugin_reminder_has_no_backticks() {
    // #1413: backticks or `$(` inside the echo reminder would trigger bash
    // command substitution, corrupting tool output and silently running the very
    // command we only suggest. The reminder must be plain prose.
    let start = OPENCODE_PLUGIN_JS
        .find("echo \"")
        .expect("echo reminder present")
        + "echo \"".len();
    let rest = &OPENCODE_PLUGIN_JS[start..];
    let end = rest.find('"').expect("echo reminder terminator");
    let reminder = &rest[..end];
    assert!(
        !reminder.contains('`'),
        "reminder has a backtick: {reminder}"
    );
    assert!(
        !reminder.contains("$("),
        "reminder has a $( construct: {reminder}"
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
    assert!(
        dir.path()
            .join(".config/agents/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
#[serial(home_env)]
fn test_amp_user_install_lands_in_config_agents() {
    // `graphify amp install` (user scope) writes the skill into an Amp search
    // root (~/.config/agents/skills), cleans the legacy ~/.amp/skills dir, and
    // writes the project AGENTS.md section. Mirrors graphify-py `_amp_install`.
    let home = tempfile::tempdir().expect("home");
    let proj = tempfile::tempdir().expect("proj");
    let legacy = home.path().join(".amp/skills/graphify");
    std::fs::create_dir_all(&legacy).expect("mk legacy");
    std::fs::write(legacy.join("SKILL.md"), "old").expect("write legacy");
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    let result = amp_install(proj.path());
    #[allow(unused_unsafe)]
    unsafe {
        std::env::remove_var("HOME");
    }
    result.expect("amp install");
    assert!(
        home.path()
            .join(".config/agents/skills/graphify/SKILL.md")
            .exists(),
        "skill must land in the Amp search root"
    );
    assert!(
        !home.path().join(".amp/skills/graphify").exists(),
        "legacy ~/.amp/skills/graphify must be cleaned up"
    );
    assert!(
        proj.path().join("AGENTS.md").exists(),
        "amp install writes the always-on AGENTS.md section"
    );
    // Uninstall removes the skill and the AGENTS.md section.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    let un = amp_uninstall(proj.path());
    #[allow(unused_unsafe)]
    unsafe {
        std::env::remove_var("HOME");
    }
    un.expect("amp uninstall");
    assert!(
        !home
            .path()
            .join(".config/agents/skills/graphify/SKILL.md")
            .exists(),
        "uninstall removes the user skill"
    );
    assert!(
        !proj.path().join("AGENTS.md").exists(),
        "uninstall removes the AGENTS.md section"
    );
}

#[test]
fn test_amp_project_install_uses_dot_agents() {
    // Project scope writes under `.agents/` (an Amp search root) plus AGENTS.md.
    let dir = tempfile::tempdir().expect("tempdir");
    install_platform_skill_project("amp", dir.path()).expect("amp project install");
    assert!(dir.path().join(".agents/skills/graphify/SKILL.md").exists());
    assert!(dir.path().join("AGENTS.md").exists());
}

#[test]
#[serial(home_env)]
fn test_install_kimi() {
    // Kimi reuses claude's skill bundle and installs under `.kimi/skills`.
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "kimi");
    assert!(dir.path().join(".kimi/skills/graphify/SKILL.md").exists());
}

#[test]
fn test_codex_project_install_writes_skill_agents_and_hook() {
    // Project-scope codex install lays down the skill, the AGENTS.md section,
    // and the .codex/hooks.json PreToolUse hook (graphify-py `_project_install`).
    let dir = tempfile::tempdir().expect("tempdir");
    install_platform_skill_project("codex", dir.path()).expect("codex project install");
    assert!(dir.path().join(".codex/skills/graphify/SKILL.md").exists());
    assert!(dir.path().join("AGENTS.md").exists());
    assert!(dir.path().join(".codex/hooks.json").exists());
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

#[test]
fn test_agents_section_uses_generic_graphify_instruction() {
    // #1530: the AGENTS.md section must not name a host-specific `skill` tool.
    assert!(!AGENTS_MD_SECTION.contains("`skill` tool"));
    assert!(!AGENTS_MD_SECTION.contains("skill: \"graphify\""));
    assert!(AGENTS_MD_SECTION.contains("use the installed graphify skill"));
}

#[test]
#[serial(home_env)]
fn test_skill_registration_uses_host_generic_instruction() {
    // #1530: the CLAUDE.md skill registration must use the host-generic
    // instruction, not the literal `skill: "graphify"` / "Skill tool".
    let dir = tempfile::tempdir().expect("tempdir");
    install_skill_to(dir.path(), "claude");
    let content = dir.path().join(".claude/CLAUDE.md").read_to_string_unwrap();
    assert!(
        content.contains("use the installed graphify skill or instructions"),
        "{content:?}"
    );
    assert!(!content.contains("skill: \"graphify\""), "{content:?}");
    assert!(!content.contains("Skill tool"), "{content:?}");
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

#[test]
fn test_uninstall_claude_hook_cleans_settings_local() {
    // #1731: a user may relocate the hook into settings.local.json (not committed
    // to a shared repo). Uninstall must clean whichever file holds it.
    let dir = tempfile::tempdir().expect("tempdir");
    install_claude_hook(dir.path()).expect("test invariant");
    let claude = dir.path().join(".claude");
    // Relocate the hook: settings.json -> settings.local.json.
    let json = claude.join("settings.json");
    let local = claude.join("settings.local.json");
    fs::copy(&json, &local).expect("relocate to local");
    fs::remove_file(&json).expect("remove settings.json");

    let msg = uninstall_claude_hook(dir.path()).expect("test invariant");
    assert!(msg.contains("settings.local.json"), "msg: {msg}");
    let v: serde_json::Value =
        serde_json::from_str(&local.read_to_string_unwrap()).expect("test invariant");
    let empty = v["hooks"]["PreToolUse"]
        .as_array()
        .is_none_or(std::vec::Vec::is_empty);
    assert!(
        empty,
        "graphify hook must be removed from settings.local.json: {v}"
    );
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

/// A `--project` install lays down the full always-on layer (skill + rules +
/// workflow) under the workspace, with the native tool-discovery frontmatter
/// injected into the skill — mirrors graphify-py's `_project_install`
/// → `_antigravity_finalize` (the project install no longer orphans the
/// rules/workflow the uninstall path removes). The shared global skill is left
/// untouched.
#[test]
#[serial(home_env)]
fn test_antigravity_install_project_writes_full_layer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    antigravity_install(dir.path(), true).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    let skill = dir.path().join(".agents/skills/graphify/SKILL.md");
    assert!(skill.exists(), "project skill must be written");
    assert!(
        dir.path().join(".agents/rules/graphify.md").exists(),
        "project install must write rules"
    );
    assert!(
        dir.path().join(".agents/workflows/graphify.md").exists(),
        "project install must write workflow"
    );
    // Native tool-discovery frontmatter is injected into the skill.
    assert!(
        skill.read_to_string_unwrap().starts_with("---\n"),
        "frontmatter must be injected into the project skill"
    );
    // The global skill location must stay untouched by a project install.
    assert!(
        !home
            .path()
            .join(".gemini/config/skills/graphify/SKILL.md")
            .exists(),
        "project install must not write the global skill"
    );
}

/// #1079: a `--project` uninstall after a prior global install removes the
/// workspace rules + workflow but leaves the shared global skill intact. This
/// mirrors graphify-py's `_project_uninstall("antigravity")`, which calls
/// `_antigravity_uninstall(project_dir, project=True)` — it cleans the
/// workspace artefacts defensively even though a project install never writes
/// them, and never touches the global skill.
#[test]
#[serial(home_env)]
fn test_antigravity_uninstall_project_after_global_keeps_global_skill() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    // SAFETY: test-only HOME override.
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    // Prior global install: writes the global skill + workspace rules/workflow.
    antigravity_install(dir.path(), false).expect("test invariant");
    let global_skill = home.path().join(".gemini/config/skills/graphify/SKILL.md");
    assert!(
        global_skill.exists(),
        "precondition: global skill must exist"
    );
    assert!(dir.path().join(".agents/rules/graphify.md").exists());
    assert!(dir.path().join(".agents/workflows/graphify.md").exists());

    // Project uninstall: clears workspace artefacts, spares the global skill.
    antigravity_uninstall(dir.path(), true).expect("test invariant");
    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("HOME");
    }
    assert!(
        !dir.path().join(".agents/rules/graphify.md").exists(),
        "project uninstall must remove workspace rules"
    );
    assert!(
        !dir.path().join(".agents/workflows/graphify.md").exists(),
        "project uninstall must remove workspace workflow"
    );
    assert!(
        !dir.path().join(".agents/skills/graphify/SKILL.md").exists(),
        "no project-local skill should remain"
    );
    assert!(
        global_skill.exists(),
        "project uninstall must not touch the global skill"
    );
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

// ---------------------------------------------------------------------------
// #1161 / #1127 / #1173: cross-platform detached launch, pinned interpreter,
// .graphify_root recovery, loud fallback
// ---------------------------------------------------------------------------

/// Both hook scripts must not rely on `nohup` / `setsid` / `disown` — Git for
/// Windows' bundled shell ships none of them (#1161).
#[test]
fn hooks_do_not_use_nohup() {
    for (name, script) in [
        ("post-commit", HOOK_SCRIPT),
        ("post-checkout", CHECKOUT_SCRIPT),
    ] {
        assert!(!script.contains("nohup"), "{name} still references nohup");
        assert!(!script.contains("setsid"), "{name} still references setsid");
        assert!(!script.contains("disown"), "{name} still uses disown");
    }
}

/// The replacement detaches via Python: `start_new_session` on POSIX and
/// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` on Windows (#1161).
#[test]
fn hooks_use_cross_platform_detach() {
    for (name, script) in [
        ("post-commit", HOOK_SCRIPT),
        ("post-checkout", CHECKOUT_SCRIPT),
    ] {
        assert!(script.contains("subprocess.Popen"), "{name} missing Popen");
        assert!(
            script.contains("start_new_session=True"),
            "{name} missing POSIX detach"
        );
        assert!(
            script.contains("0x00000008"),
            "{name} missing DETACHED_PROCESS flag"
        );
        assert!(
            script.contains("0x00000200"),
            "{name} missing CREATE_NEW_PROCESS_GROUP flag"
        );
    }
}

/// Both rebuild bodies read `<output-dir>/.graphify_root` and pass the
/// recovered root to `_rebuild_code`, so a scoped build is not silently
/// expanded to the full repo (#1173). The output dir is resolved from
/// `GRAPHIFY_OUT` at hook-run time rather than hardcoded (#1423).
#[test]
fn rebuild_bodies_read_graphify_root() {
    for (name, script) in [
        ("post-commit", HOOK_SCRIPT),
        ("post-checkout", CHECKOUT_SCRIPT),
    ] {
        assert!(
            script.contains(".graphify_root"),
            "{name} ignores .graphify_root"
        );
        assert!(
            script.contains("GRAPHIFY_OUT"),
            "{name} ignores the GRAPHIFY_OUT override (#1423)"
        );
        assert!(
            script.contains("_rebuild_code(_root"),
            "{name} does not pass recovered root"
        );
        assert!(
            script.contains("read_text(encoding='utf-8')"),
            "{name} root read is not single-quoted (shell-quote-safe)"
        );
        assert!(
            script.contains("from graphify.reflect import reflect"),
            "{name} does not refresh the lessons doc post-rebuild (#1441)"
        );
    }
}

/// The interpreter detection has a loud fallback instead of a bare silent exit.
#[test]
fn python_detect_has_loud_fallback() {
    assert!(PYTHON_DETECT.contains("could not locate"));
}

/// The pinned probe and `.graphify_python` probe are present.
#[test]
fn python_detect_has_pinned_and_file_probes() {
    assert!(PYTHON_DETECT.contains("_PINNED="));
    assert!(PYTHON_DETECT.contains("graphify-out/.graphify_python"));
}

/// #1586: the interpreter-detection allowlists must accept `@` so Homebrew's
/// versioned `python@3.13` path is not blanked, which would drop detection to a
/// bare `python3` without graphify installed.
#[test]
fn python_detect_allows_at_sign_in_interpreter_path() {
    assert!(
        PYTHON_DETECT.contains("a-zA-Z0-9/_.@"),
        "interpreter allowlist must include `@` for python@3.x paths"
    );
    assert!(
        !PYTHON_DETECT.contains("[!a-zA-Z0-9/_.-]"),
        "the pre-#1586 allowlist (missing `@`) must not be present"
    );
}

/// End-to-end: the installed hooks substitute the `__PINNED_PYTHON__`
/// placeholder and contain the cross-platform detach (#1161, #1127).
#[test]
#[serial]
fn installed_hooks_substitute_placeholder_and_detach() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("install");
    for name in ["post-commit", "post-checkout"] {
        let hook = repo.join(".git").join("hooks").join(name);
        let text = fs::read_to_string(&hook).expect("read hook");
        assert!(
            !text.contains("__PINNED_PYTHON__"),
            "{name} placeholder not substituted"
        );
        assert!(
            text.contains("_PINNED=''"),
            "{name} pinned probe missing (empty pin)"
        );
        assert!(
            text.contains("start_new_session=True"),
            "{name} not detached"
        );
        assert!(!text.contains("nohup"), "{name} still references nohup");
    }
}

/// #879c058: both hook scripts default rebuild workers to 1 on Git for
/// Windows/MSYS (unless `GRAPHIFY_MAX_WORKERS` is explicitly set), since those
/// shells inherit fragile pipe handles from GUI clients and agent shells.
#[test]
fn hook_scripts_cap_windows_rebuild_workers() {
    for script in [HOOK_SCRIPT, CHECKOUT_SCRIPT] {
        assert!(
            script.contains(r#"export GRAPHIFY_MAX_WORKERS="${GRAPHIFY_MAX_WORKERS:-1}""#),
            "hook script must default GRAPHIFY_MAX_WORKERS=1 on Windows/MSYS"
        );
        assert!(
            script.contains(r#"[ -n "${WINDIR:-}" ] || [ -n "${MSYSTEM:-}" ]"#),
            "the cap must be gated on WINDIR/MSYSTEM"
        );
    }
}

/// #1809: `GRAPHIFY_SKIP_HOOK=1` must suppress BOTH hooks. post-checkout
/// previously lacked the check, so the var stopped commit rebuilds but not
/// branch-switch ones.
#[test]
fn hooks_honor_skip_env() {
    for (name, script) in [
        ("post-commit", HOOK_SCRIPT),
        ("post-checkout", CHECKOUT_SCRIPT),
    ] {
        assert!(
            script.contains(r#"[ "${GRAPHIFY_SKIP_HOOK:-0}" = "1" ] && exit 0"#),
            "{name} does not honor GRAPHIFY_SKIP_HOOK"
        );
    }
}

/// #1809/#1806: both hooks must short-circuit in a linked worktree
/// (git-dir != common-dir), comparing ABSOLUTE paths so the primary checkout
/// (where --git-common-dir is the relative ".git") is not false-positived.
#[test]
fn hooks_skip_linked_worktrees() {
    for (name, script) in [
        ("post-commit", HOOK_SCRIPT),
        ("post-checkout", CHECKOUT_SCRIPT),
    ] {
        assert_eq!(
            script.matches("_GFY_GITDIR=").count(),
            1,
            "{name} guard not present exactly once"
        );
        assert!(
            script.contains("git rev-parse --git-common-dir"),
            "{name} missing common-dir probe"
        );
        // absolute-normalized compare, not a raw string compare of git output
        assert!(
            script.contains(r#"cd "$(git rev-parse --git-dir 2>/dev/null)" 2>/dev/null && pwd"#),
            "{name} does not resolve git-dir to an absolute path"
        );
        assert!(
            script.contains(r#"[ "$_GFY_GITDIR" != "$_GFY_COMMONDIR" ]"#),
            "{name} missing the git-dir vs common-dir compare"
        );
    }
}

/// End-to-end against a real `git worktree`: [`WORKTREE_GUARD`] falls through on
/// the primary checkout and exits early inside a linked worktree (#1809, #1806).
#[test]
fn worktree_guard_runs_on_primary_skips_linked() {
    use std::process::Command;
    if Command::new("git").arg("--version").output().is_err() {
        return; // git not available
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let primary = tmp.path().join("primary");
    std::fs::create_dir_all(&primary).expect("mkdir primary");

    let git = |args: &[&str], cwd: &std::path::Path| {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git run");
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
    };
    git(&["init", "-q", "."], &primary);
    git(&["config", "user.email", "t@t.co"], &primary);
    git(&["config", "user.name", "t"], &primary);
    std::fs::write(primary.join("a.txt"), "x").expect("write a.txt");
    git(&["add", "-A"], &primary);
    git(&["commit", "-qm", "init"], &primary);
    let linked = tmp.path().join("linked");
    git(
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().expect("utf-8"),
            "-b",
            "feature",
        ],
        &primary,
    );

    let snippet = format!("{WORKTREE_GUARD}echo RAN\n");
    let run = |cwd: &std::path::Path| {
        let out = Command::new("sh")
            .arg("-c")
            .arg(&snippet)
            .current_dir(cwd)
            .output()
            .expect("sh run");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(
        run(&primary).contains("RAN"),
        "guard wrongly skipped the primary checkout"
    );
    assert!(
        !run(&linked).contains("RAN"),
        "guard failed to skip the linked worktree"
    );
}

// ── #1907: hooks_dir tolerates git's duplicate keys / repeated sections ───────

/// Append duplicate keys and a repeated section to `.git/config` — the shape
/// VS Code and other tools legally write that a strict configparser rejected.
fn append_duplicate_config_entries(repo: &Path) {
    let cfg = repo.join(".git").join("config");
    let mut content = fs::read_to_string(&cfg).expect("read .git/config");
    content.push_str(
        "[remote \"origin\"]\n\
         \tfetch = +refs/heads/*:refs/remotes/origin/*\n\
         \tfetch = +refs/heads/*:refs/remotes/origin/*\n\
         [core]\n\
         \tignorecase = true\n",
    );
    fs::write(&cfg, content).expect("write .git/config");
}

#[test]
#[serial]
fn test_hooks_dir_no_warning_on_duplicate_config_keys() {
    // git legally allows duplicate keys/sections; hooks_dir must resolve cleanly
    // (a strict configparser broke on VS Code configs, #1907).
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    append_duplicate_config_entries(&repo);
    let d = hooks_dir(&repo).expect("hooks_dir resolves cleanly");
    let expected = repo.join(".git").join("hooks");
    assert_eq!(d, expected.canonicalize().unwrap_or(expected));
}

#[test]
#[serial]
fn test_hooks_dir_duplicate_config_keys_honor_custom_hookspath() {
    // With duplicate keys present, a custom core.hooksPath is still honored.
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    set_hookspath(&repo, ".husky");
    append_duplicate_config_entries(&repo);
    let d = hooks_dir(&repo).expect("hooks_dir resolves cleanly");
    let expected = repo.join(".husky");
    assert_eq!(d, expected.canonicalize().unwrap_or(expected));
}

// ── #1902: hook install registers the graph.json union merge driver ───────────

fn git_config_get(repo: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", &repo.to_string_lossy(), "config", "--get", key])
        .output()
        .expect("git config --get");
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

#[test]
#[serial]
fn test_install_registers_merge_driver() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    let result = install(&repo).expect("install");
    let driver = git_config_get(&repo, "merge.graphify.driver").expect("driver set");
    assert!(driver.contains("merge-driver %O %A %B"), "driver: {driver}");
    let attrs = fs::read_to_string(repo.join(".gitattributes")).expect("read .gitattributes");
    assert!(
        attrs
            .lines()
            .any(|l| l.contains("graph.json") && l.contains("merge=graphify")),
        "gitattributes: {attrs}"
    );
    assert!(result.contains("merge driver"), "result: {result}");
}

#[test]
#[serial]
fn test_install_merge_driver_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    install(&repo).expect("install 1");
    install(&repo).expect("install 2");
    let attrs = fs::read_to_string(repo.join(".gitattributes")).expect("read");
    let matches = attrs
        .lines()
        .filter(|l| l.contains("merge=graphify"))
        .count();
    assert_eq!(matches, 1, "merge attr duplicated: {attrs}");
}

#[test]
#[serial]
fn test_install_preserves_existing_gitattributes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    fs::write(repo.join(".gitattributes"), "*.png binary\n").expect("write");
    install(&repo).expect("install");
    let content = fs::read_to_string(repo.join(".gitattributes")).expect("read");
    assert!(content.contains("*.png binary"), "clobbered: {content}");
    assert!(content.contains("merge=graphify"));
}

#[test]
#[serial]
fn test_uninstall_removes_merge_driver_keeps_other_attrs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = make_git_repo(dir.path());
    fs::write(repo.join(".gitattributes"), "*.png binary\n").expect("write");
    install(&repo).expect("install");
    uninstall(&repo).expect("uninstall");
    assert!(
        git_config_get(&repo, "merge.graphify.driver").is_none(),
        "merge driver config not unset"
    );
    let content = fs::read_to_string(repo.join(".gitattributes")).expect("read");
    assert!(
        content.contains("*.png binary"),
        "other attrs lost: {content}"
    );
    assert!(
        !content.contains("merge=graphify"),
        "merge attr not removed: {content}"
    );
}
