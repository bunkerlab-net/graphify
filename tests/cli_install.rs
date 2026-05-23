//! Coverage tests for the `install`/`uninstall`/platform CLI subcommands.
//!
//! These run uninstall on a tempdir (no-op when nothing was installed) so they
//! exercise the dispatch and platform plumbing without touching the user's
//! actual config directories.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use assert_cmd::Command;

fn cli() -> Command {
    Command::cargo_bin("graphify").expect("cargo-bin graphify")
}

fn uninstall_runs(platform: &str) {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
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
