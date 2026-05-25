//! Coverage tests for `graphify export` subcommands and related commands.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn cli() -> Command {
    Command::cargo_bin("graphify").expect("cargo-bin graphify")
}

fn write_graph_json(path: &Path) {
    fs::write(
        path,
        r#"{"nodes":[
            {"id":"a","label":"A","file_type":"code","source_file":"a.py","community":0},
            {"id":"b","label":"B","file_type":"code","source_file":"b.py","community":1}
        ],"edges":[
            {"source":"a","target":"b","context":"CALLS","confidence":"EXTRACTED"}
        ]}"#,
    )
    .unwrap();
}

fn write_analysis_json(path: &Path) {
    fs::write(
        path,
        r#"{
            "root": ".",
            "communities": {"0": ["a"], "1": ["b"]},
            "cohesion": {"0": 1.0, "1": 1.0},
            "cohesion_scores": {"0": 1.0, "1": 1.0},
            "gods": [],
            "god_nodes": [],
            "surprises": [],
            "surprising_connections": [],
            "suggested_questions": [],
            "tokens": {"input": 0, "output": 0},
            "min_community_size": 3
        }"#,
    )
    .unwrap();
}

// ── export wiki ────────────────────────────────────────────────────────────

#[test]
fn export_wiki_with_analysis() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);
    write_analysis_json(&out.join(".graphify_analysis.json"));

    cli()
        .arg("export")
        .arg("wiki")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    assert!(out.join("wiki").exists());
}

// ── export callflow-html ───────────────────────────────────────────────────

#[test]
fn export_callflow_html_writes_output() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);
    write_analysis_json(&out.join(".graphify_analysis.json"));

    cli()
        .arg("export")
        .arg("callflow-html")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
}

// ── merge-graphs ───────────────────────────────────────────────────────────

#[test]
fn merge_graphs_unions_two_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    write_graph_json(&a);
    write_graph_json(&b);
    let out = dir.path().join("merged.json");

    cli()
        .arg("merge-graphs")
        .arg(&a)
        .arg(&b)
        .arg("--out")
        .arg(&out)
        .assert()
        .success();
    assert!(out.exists());
}

// ── merge-driver three-way ─────────────────────────────────────────────────

#[test]
fn merge_driver_three_way() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.json");
    let cur = dir.path().join("current.json");
    let other = dir.path().join("other.json");
    write_graph_json(&base);
    write_graph_json(&cur);
    write_graph_json(&other);

    cli()
        .arg("merge-driver")
        .arg(&base)
        .arg(&cur)
        .arg(&other)
        .assert()
        .success();
}

// ── tree --depth and various flags ────────────────────────────────────────

#[test]
fn tree_renders_with_graph_only() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph_path);
    cli()
        .arg("tree")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
}

// ── query / path / explain ─────────────────────────────────────────────────

#[test]
fn query_runs_on_graph() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph_path);
    cli()
        .arg("query")
        .arg("how does A relate to B")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
}

#[test]
fn path_runs_on_graph() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph_path);
    cli()
        .arg("path")
        .arg("a")
        .arg("b")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
}

#[test]
fn explain_runs_on_graph() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph_path);
    cli()
        .arg("explain")
        .arg("a")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
}

// ── error paths ────────────────────────────────────────────────────────────

#[test]
fn validate_errors_on_missing_file() {
    cli()
        .arg("validate")
        .arg("/nonexistent/path.json")
        .assert()
        .failure();
}

#[test]
fn merge_chunks_warns_on_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.json");
    cli()
        .arg("merge-chunks")
        .arg("/nonexistent/chunk.json")
        .arg("--out")
        .arg(&out)
        .assert()
        .success()
        .stderr(contains("warning: skipping"));
}

// ── update command (rebuild_code wrapper) ──────────────────────────────────

#[test]
fn update_no_cluster_path() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src").join("foo.py"),
        "def foo():\n    pass\n",
    )
    .unwrap();

    cli()
        .arg("update")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();
    assert!(dir.path().join("graphify-out").join("graph.json").exists());
}

#[test]
fn update_with_force() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src").join("foo.py"),
        "def foo():\n    pass\n",
    )
    .unwrap();
    cli()
        .arg("update")
        .arg(dir.path())
        .arg("--force")
        .arg("--no-cluster")
        .assert()
        .success();
}

// ── export html: communities fallback when analysis sidecar is absent ─────

/// The watch / post-commit rebuild path only regenerates `graph.json` +
/// `GRAPH_REPORT.md`; `.graphify_analysis.json` is left stale or absent. Some
/// skill workflows also clean up temp files after `graphify extract`. In both
/// cases the per-node `community` attribute on `graph.json` (written by
/// `to_json`) is intact, but pre-d778e2c every downstream export would silently
/// produce a degraded artifact. These tests pin the fallback so the reconstruction
/// always happens.
///
/// Ports `tests/test_cli_export.py::test_export_html_falls_back_to_node_community_attribute`.
#[test]
fn export_html_falls_back_to_node_community_attribute() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out)?;
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);
    // Simulate the watch-rebuild / cleanup case: graph.json survives, the
    // analysis sidecar is missing entirely.
    assert!(!out.join(".graphify_analysis.json").exists());

    cli()
        .arg("export")
        .arg("html")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    assert!(
        out.join("graph.html").exists(),
        "graph.html should be generated from the fallback"
    );
    assert!(out.join("graph.html").metadata()?.len() > 0);
    Ok(())
}

/// Ports `tests/test_cli_export.py::test_export_html_fallback_recovers_multiple_communities`.
#[test]
fn export_html_fallback_recovers_multiple_communities() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out)?;
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);

    // Count distinct `community` values on the graph nodes — this is what the
    // fallback will reconstruct.
    let graph: serde_json::Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
    let cids: std::collections::HashSet<i64> = graph["nodes"]
        .as_array()
        .ok_or("graph.json `nodes` field missing or not an array")?
        .iter()
        .filter_map(|n| n.get("community").and_then(serde_json::Value::as_i64))
        .collect();
    assert_eq!(cids.len(), 2, "fixture should have 2 distinct communities");

    cli()
        .arg("export")
        .arg("html")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    assert!(out.join("graph.html").exists());
    Ok(())
}

/// Ports `tests/test_cli_export.py::test_export_html_no_community_data_at_all_still_succeeds`.
#[test]
fn export_html_no_community_data_at_all_still_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out)?;
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);

    // Strip the `community` attribute from every node — emulates a hand-built
    // graph.json or an older `to_json`.
    let mut graph: serde_json::Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
    let nodes = graph["nodes"]
        .as_array_mut()
        .ok_or("graph.json `nodes` field missing or not an array")?;
    for n in nodes {
        if let Some(obj) = n.as_object_mut() {
            obj.remove("community");
        }
    }
    fs::write(&graph_path, serde_json::to_string(&graph)?)?;

    // Should NOT crash. The renderer may emit an empty / degraded view, but
    // the exit code stays clean.
    cli()
        .arg("export")
        .arg("html")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    Ok(())
}

// ── path/explain on missing graph ──────────────────────────────────────────

#[test]
fn path_errors_when_graph_missing() {
    cli()
        .arg("path")
        .arg("from")
        .arg("to")
        .arg("--graph")
        .arg("/nonexistent/graph.json")
        .assert()
        .failure()
        .stderr(
            contains("graph.json")
                .or(contains("not found"))
                .or(contains("No such")),
        );
}
