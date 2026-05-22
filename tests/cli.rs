//! Workspace-level CLI integration tests.
//!
//! Each test spawns the built `graphify` binary and asserts on stdout/stderr.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn cli() -> Command {
    Command::cargo_bin("graphify").expect("cargo-bin graphify")
}

#[test]
fn version_flag_prints_version() {
    cli()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains("graphify"));
}

#[test]
fn validate_accepts_minimal_extraction_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extraction.json");
    fs::write(&path, r#"{"nodes":[],"edges":[],"hyperedges":[]}"#).unwrap();

    cli()
        .arg("validate")
        .arg(&path)
        .assert()
        .success()
        .stdout(contains("OK"));
}

#[test]
fn validate_rejects_missing_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    fs::write(&path, r#"{"nodes":[]}"#).unwrap();

    cli().arg("validate").arg(&path).assert().failure();
}

#[test]
fn hook_status_runs_without_repo() {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
        .arg("hook")
        .arg("status")
        .assert()
        .success();
}

#[test]
fn global_path_prints_expected_location() {
    cli()
        .arg("global")
        .arg("path")
        .assert()
        .success()
        .stdout(contains("global-graph.json"));
}

#[test]
fn help_prints_subcommands() {
    cli().arg("--help").assert().success().stdout(
        contains("validate")
            .and(contains("hook"))
            .and(contains("global")),
    );
}

#[test]
fn unknown_subcommand_fails() {
    cli()
        .arg("definitely-not-a-real-subcommand")
        .assert()
        .failure();
}

#[test]
fn path_command_errors_on_missing_graph() {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
        .arg("path")
        .arg("foo")
        .arg("bar")
        .assert()
        .failure();
}

#[test]
fn benchmark_errors_without_graph() {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
        .arg("benchmark")
        .assert()
        .failure();
}

#[test]
fn query_command_errors_on_missing_graph() {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .current_dir(dir.path())
        .arg("query")
        .arg("what does foo do")
        .assert()
        .failure();
}

#[test]
fn validate_round_trip_simple_extraction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ex.json");
    fs::write(
        &path,
        r#"{"nodes":[{"id":"a","label":"A","file_type":"code","source_file":"a.py"}],"edges":[],"hyperedges":[]}"#,
    )
    .unwrap();
    cli().arg("validate").arg(&path).assert().success();
}

#[test]
fn merge_graphs_unions_two_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    fs::write(
        &a,
        r#"{"nodes":[{"id":"a","label":"A","file_type":"code","source_file":"a.py"}],"edges":[]}"#,
    )
    .unwrap();
    fs::write(
        &b,
        r#"{"nodes":[{"id":"b","label":"B","file_type":"code","source_file":"b.py"}],"edges":[]}"#,
    )
    .unwrap();
    let out = dir.path().join("merged.json");
    cli()
        .arg("merge-graphs")
        .arg(&a)
        .arg(&b)
        .arg("--out")
        .arg(&out)
        .assert()
        .success();
    let body = fs::read_to_string(&out).unwrap();
    assert!(body.contains(r#""id": "a""#));
    assert!(body.contains(r#""id": "b""#));
}

#[test]
fn merge_driver_writes_to_current() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.json");
    let current = dir.path().join("current.json");
    let other = dir.path().join("other.json");
    let empty = r#"{"nodes":[],"edges":[]}"#;
    fs::write(&base, empty).unwrap();
    fs::write(
        &current,
        r#"{"nodes":[{"id":"a","label":"A","file_type":"code","source_file":"a.py"}],"edges":[]}"#,
    )
    .unwrap();
    fs::write(
        &other,
        r#"{"nodes":[{"id":"b","label":"B","file_type":"code","source_file":"b.py"}],"edges":[]}"#,
    )
    .unwrap();
    cli()
        .arg("merge-driver")
        .arg(&base)
        .arg(&current)
        .arg(&other)
        .assert()
        .success();
    let body = fs::read_to_string(&current).unwrap();
    assert!(body.contains(r#""id": "a""#));
    assert!(body.contains(r#""id": "b""#));
}

#[test]
fn export_graphml_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    fs::write(
        &graph_path,
        r#"{"nodes":[{"id":"a","label":"A","file_type":"code","source_file":"a.py"},{"id":"b","label":"B","file_type":"code","source_file":"b.py"}],"edges":[{"source":"a","target":"b","context":"CALLS","confidence":"EXTRACTED"}]}"#,
    )
    .unwrap();
    cli()
        .arg("export")
        .arg("graphml")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    assert!(dir.path().join("graph.graphml").exists());
}
