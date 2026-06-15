//! Coverage tests for the `install`/`uninstall`/platform CLI subcommands.
//!
//! These run uninstall on a tempdir (no-op when nothing was installed) so they
//! exercise the dispatch and platform plumbing without touching the user's
//! actual config directories.

// File-top `expect_used`/`unwrap_used` suppression is the sanctioned project
// convention for integration-test files (AGENTS.md "Strict lints"): a panic in
// a CLI-test fixture (e.g. `tempdir()` failing) is itself a test failure, so the
// blanket allow is kept rather than threading `Result` through every `#[test]`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use assert_cmd::Command;

fn cli() -> Command {
    Command::cargo_bin("graphify").expect("cargo-bin graphify")
}

fn uninstall_runs(platform: &str) {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
        // Isolate HOME so any user-scope artifact removal (skills, commands,
        // config for Claude/Gemini/Kilo/etc.) stays inside the temp dir and
        // never touches the developer's real directories.
        .env("HOME", dir.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .arg(platform)
        .arg("uninstall")
        .assert()
        .success();
}

#[test]
fn claude_uninstall_runs() {
    uninstall_runs("claude");
}

#[test]
fn gemini_uninstall_runs() {
    uninstall_runs("gemini");
}

#[test]
fn cursor_uninstall_runs() {
    uninstall_runs("cursor");
}

#[test]
fn vscode_uninstall_runs() {
    uninstall_runs("vscode");
}

#[test]
fn kiro_uninstall_runs() {
    uninstall_runs("kiro");
}

#[test]
fn kilo_uninstall_runs() {
    uninstall_runs("kilo");
}

#[test]
fn kilo_install_runs_and_writes_artifacts() {
    // `graphify kilo install` writes the global skill/command (under HOME) and
    // the always-on project wiring (AGENTS.md + .kilo plugin) under cwd.
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    cli()
        .current_dir(project.path())
        .env("HOME", home.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .arg("kilo")
        .arg("install")
        .assert()
        .success();
    assert!(
        home.path()
            .join(".config/kilo/command/graphify.md")
            .exists()
    );
    assert!(project.path().join("AGENTS.md").exists());
    assert!(project.path().join(".kilo/plugins/graphify.js").exists());
}

#[test]
fn antigravity_uninstall_runs() {
    uninstall_runs("antigravity");
}

#[test]
fn opencode_uninstall_runs() {
    uninstall_runs("opencode");
}

#[test]
fn aider_uninstall_runs() {
    uninstall_runs("aider");
}

#[test]
fn claw_uninstall_runs() {
    uninstall_runs("claw");
}

#[test]
fn droid_uninstall_runs() {
    uninstall_runs("droid");
}

#[test]
fn codex_uninstall_runs() {
    uninstall_runs("codex");
}

#[test]
fn trae_uninstall_runs() {
    uninstall_runs("trae");
}

#[test]
fn hermes_uninstall_runs() {
    uninstall_runs("hermes");
}

#[test]
fn uninstall_command_runs_globally() {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .arg("uninstall")
        .assert()
        .success();
}

#[test]
fn uninstall_purge_removes_graphify_out() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("graphify-out")).unwrap();
    std::fs::write(dir.path().join("graphify-out").join("graph.json"), "{}").unwrap();
    cli()
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .arg("uninstall")
        .arg("--purge")
        .assert()
        .success();
    assert!(!dir.path().join("graphify-out").exists());
}

#[test]
fn hook_check_runs_silently() {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
        .arg("hook-check")
        .assert()
        .success();
}

#[test]
fn codebuddy_uninstall_runs() {
    uninstall_runs("codebuddy");
}

#[test]
fn codebuddy_install_writes_artifacts() {
    // `graphify codebuddy install` writes CODEBUDDY.md + the .codebuddy hook
    // under cwd and the skill under the project's .codebuddy tree (#1136).
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    cli()
        .current_dir(project.path())
        .env("HOME", home.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .arg("codebuddy")
        .arg("install")
        .assert()
        .success();
    assert!(project.path().join("CODEBUDDY.md").exists());
    assert!(project.path().join(".codebuddy/settings.json").exists());
    assert!(
        project
            .path()
            .join(".codebuddy/skills/graphify/SKILL.md")
            .exists()
    );
}

#[test]
fn uninstall_all_removes_codebuddy_artifacts() {
    // `graphify codebuddy install` then `graphify uninstall` must clean up
    // CODEBUDDY.md and the .codebuddy/settings.json hook (#1136).
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    cli()
        .current_dir(project.path())
        .env("HOME", home.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .arg("codebuddy")
        .arg("install")
        .assert()
        .success();
    assert!(project.path().join("CODEBUDDY.md").exists());

    cli()
        .current_dir(project.path())
        .env("HOME", home.path())
        .env_remove("CLAUDE_CONFIG_DIR")
        .arg("uninstall")
        .assert()
        .success();
    assert!(!project.path().join("CODEBUDDY.md").exists());

    // The graphify hook entries are stripped; removing them empties the
    // PreToolUse array so no "graphify" reference remains in the raw settings.
    let settings_path = project.path().join(".codebuddy/settings.json");
    if settings_path.exists() {
        let raw = std::fs::read_to_string(&settings_path).unwrap();
        assert!(!raw.contains("graphify"));
    }
}

#[test]
fn codebuddy_listed_in_help() {
    cli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("codebuddy"));
}
