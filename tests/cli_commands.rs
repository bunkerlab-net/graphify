//! Coverage tests for the `graphify` CLI subcommands.
//!
//! These exercise the dispatcher and per-command handlers via the real binary
//! (no LLM backend is configured, so semantic phases short-circuit).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn cli() -> Command {
    Command::cargo_bin("graphify").expect("cargo-bin graphify")
}

/// A [`cli`] command with every backend-selection env var scrubbed, so
/// `graphify_llm::detect_backend()` cannot pick up a real LLM key from the
/// developer's (or CI's) environment and turn an AST-only smoke test into a
/// live API call. Covers the full set of vars `detect_backend` inspects:
/// the per-provider API keys, the Bedrock credential-provider vars, and the
/// Ollama base URL — plus the graphify-specific overrides, defensively.
fn cli_no_backend() -> Command {
    let mut cmd = cli();
    for key in [
        // API keys (gemini -> kimi -> claude -> openai -> deepseek).
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "MOONSHOT_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        // Bedrock credential-provider vars.
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_PROFILE",
        "AWS_WEB_IDENTITY_TOKEN_FILE",
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI",
        // Ollama (checked last).
        "OLLAMA_BASE_URL",
        // Defensive: graphify-specific overrides.
        "GRAPHIFY_BACKEND",
        "GRAPHIFY_OPENAI_MODEL",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

/// Write a small Python project plus a runnable graph.json fixture into `dir`.
fn write_python_project(dir: &Path) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src").join("main.py"),
        "class A:\n    def hello(self):\n        return 'hi'\n\ndef main():\n    A().hello()\n",
    )
    .unwrap();
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

// ── extract command (AST-only path; no backend env vars) ───────────────────

#[test]
fn extract_runs_without_backend_writes_graph() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("MOONSHOT_API_KEY")
        .env_remove("DEEPSEEK_API_KEY")
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("OLLAMA_BASE_URL")
        .env_remove("GRAPHIFY_BACKEND")
        .env_remove("GRAPHIFY_OPENAI_MODEL")
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();

    assert!(
        dir.path().join("graphify-out").join("graph.json").exists(),
        "graph.json not written"
    );
}

#[test]
fn extract_mode_deep_prints_banner_and_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    // No backend is configured (scrubbed), so deep mode degrades to an AST-only
    // run and the banner reports that rather than implying semantic enrichment.
    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--mode")
        .arg("deep")
        .arg("--no-cluster")
        .assert()
        .success()
        .stderr(contains("deep mode"));

    assert!(dir.path().join("graphify-out").join("graph.json").exists());
}

#[test]
fn extract_mode_invalid_value_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli()
        .arg("extract")
        .arg(dir.path())
        .arg("--mode")
        .arg("bogus")
        .assert()
        .failure()
        .code(2)
        .stderr(contains("invalid value").or(contains("'bogus'")));
}

#[test]
fn extract_with_custom_out_dir() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    let out = dir.path().join("custom-out");
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("MOONSHOT_API_KEY")
        .env_remove("DEEPSEEK_API_KEY")
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("OLLAMA_BASE_URL")
        .env_remove("GRAPHIFY_BACKEND")
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .arg("--out")
        .arg(&out)
        .assert()
        .success();
    assert!(out.join("graph.json").exists());
}

#[test]
fn extract_incremental_mode_with_existing_manifest() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    // First run produces graphify-out/manifest.json + graph.json
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .arg("extract")
        .arg(dir.path())
        .assert()
        .success();
    assert!(
        dir.path()
            .join("graphify-out")
            .join("manifest.json")
            .exists()
    );
    // Second run takes the incremental path.
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .arg("extract")
        .arg(dir.path())
        .assert()
        .success()
        .stderr(contains("incremental scan"));
}

#[test]
fn extract_default_mode_runs_full_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .env_remove("GRAPHIFY_BACKEND")
        .arg("extract")
        .arg(dir.path())
        .assert()
        .success();
    // Full pipeline writes graph.json + GRAPH_REPORT.md.
    let out = dir.path().join("graphify-out");
    assert!(out.join("graph.json").exists());
    assert!(out.join("GRAPH_REPORT.md").exists());
}

#[test]
fn extract_with_excludes() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    std::fs::create_dir_all(dir.path().join("vendor")).unwrap();
    std::fs::write(
        dir.path().join("vendor").join("dep.py"),
        "def vendor_thing(): pass\n",
    )
    .unwrap();
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .arg("--exclude")
        .arg("vendor/**")
        .assert()
        .success();
    let graph_path = dir.path().join("graphify-out").join("graph.json");
    let text = fs::read_to_string(&graph_path).unwrap();
    assert!(
        !text.contains("vendor_thing"),
        "vendor file should be excluded"
    );
}

#[test]
fn extract_with_resolution_and_exclude_hubs() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .arg("extract")
        .arg(dir.path())
        .arg("--resolution")
        .arg("1.5")
        .arg("--exclude-hubs")
        .arg("0.95")
        .assert()
        .success();
}

#[test]
fn extract_no_cluster_skips_html_phase() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli()
        .env_remove("GEMINI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .env_remove("GRAPHIFY_BACKEND")
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();
    // With no-cluster, no GRAPH_REPORT.md is written.
    assert!(
        !dir.path()
            .join("graphify-out")
            .join("GRAPH_REPORT.md")
            .exists()
    );
}

// ── cluster-only command ───────────────────────────────────────────────────

#[test]
fn cluster_only_runs_on_existing_graph() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);
    cli()
        .arg("cluster-only")
        .arg(dir.path())
        .arg("--graph")
        .arg(&graph_path)
        .arg("--no-viz")
        .assert()
        .success();
}

// ── merge-chunks ──────────────────────────────────────────────────────────

#[test]
fn merge_chunks_combines_files() {
    let dir = tempfile::tempdir().unwrap();
    let chunk_a = dir.path().join("a.json");
    let chunk_b = dir.path().join("b.json");
    fs::write(
        &chunk_a,
        r#"{"nodes":[{"id":"x","label":"X","file_type":"code","source_file":"x.py"}],"edges":[]}"#,
    )
    .unwrap();
    fs::write(
        &chunk_b,
        r#"{"nodes":[{"id":"y","label":"Y","file_type":"code","source_file":"y.py"}],"edges":[]}"#,
    )
    .unwrap();
    let out = dir.path().join("merged.json");
    cli()
        .arg("merge-chunks")
        .arg(&chunk_a)
        .arg(&chunk_b)
        .arg("--out")
        .arg(&out)
        .assert()
        .success();
    assert!(out.exists());
    let merged: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(merged["nodes"].as_array().unwrap().len(), 2);
}

// ── tree ───────────────────────────────────────────────────────────────────

#[test]
fn tree_command_works_on_graph() {
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

// ── export sub-commands ────────────────────────────────────────────────────

#[test]
fn export_html_skipped_with_no_viz() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);
    write_analysis_json(&out.join(".graphify_analysis.json"));

    cli()
        .arg("export")
        .arg("html")
        .arg("--graph")
        .arg(&graph_path)
        .arg("--no-viz")
        .assert()
        .success();
}

#[test]
fn export_obsidian_writes_notes() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);
    write_analysis_json(&out.join(".graphify_analysis.json"));

    let dest = dir.path().join("obs");
    cli()
        .arg("export")
        .arg("obsidian")
        .arg("--graph")
        .arg(&graph_path)
        .arg("--out")
        .arg(&dest)
        .assert()
        .success();
    assert!(dest.exists(), "obsidian dir not created");
}

#[test]
fn export_svg_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);
    write_analysis_json(&out.join(".graphify_analysis.json"));

    cli()
        .arg("export")
        .arg("svg")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    assert!(out.join("graph.svg").exists());
}

#[test]
fn export_neo4j_writes_cypher() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);

    cli()
        .arg("export")
        .arg("neo4j")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    // The Rust impl writes `<out_dir>/cypher.txt`.
    assert!(
        out.join("cypher.txt").exists(),
        "no cypher.txt found in {}",
        out.display()
    );
}

// ── benchmark ──────────────────────────────────────────────────────────────

#[test]
fn benchmark_runs_on_graph() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    write_graph_json(&graph_path);
    cli().arg("benchmark").arg(&graph_path).assert().success();
}

// ── cache-check ────────────────────────────────────────────────────────────

#[test]
fn cache_check_runs() {
    let dir = tempfile::tempdir().unwrap();
    let files_list = dir.path().join("files.txt");
    fs::write(&files_list, "src/a.py\n").unwrap();
    cli()
        .current_dir(dir.path())
        .arg("cache-check")
        .arg(&files_list)
        .arg("--root")
        .arg(dir.path())
        .assert()
        .success();
}

// ── hook --help/status ─────────────────────────────────────────────────────

#[test]
fn hook_help_lists_subcommands() {
    cli()
        .arg("hook")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("status").or(contains("install")));
}

// ── prs --help ─────────────────────────────────────────────────────────────

#[test]
fn prs_help() {
    cli().arg("prs").arg("--help").assert().success();
}

// ── check-update prints flag-aware result ─────────────────────────────────

#[test]
fn check_update_prints_status() {
    let dir = tempfile::tempdir().unwrap();
    // Create a graphify-out + needs_update flag so check_update has something to inspect.
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("needs_update"), "1").unwrap();
    cli().arg("check-update").arg(dir.path()).assert().success();
}
