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
    cli_no_backend()
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
fn extract_cargo_flag_merges_crate_nodes() {
    // `--cargo` introspects Cargo.toml manifests and merges `crate:<name>`
    // nodes + `crate_depends_on` edges into the graph (#1271).
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"foo\", \"bar\"]\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("foo")).unwrap();
    fs::write(
        dir.path().join("foo").join("Cargo.toml"),
        "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n\n[dependencies]\nbar = { path = \"../bar\" }\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("bar")).unwrap();
    fs::write(
        dir.path().join("bar").join("Cargo.toml"),
        "[package]\nname = \"bar\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--cargo")
        .arg("--no-cluster")
        .assert()
        .success()
        .stderr(contains("Cargo:"));

    let raw = fs::read_to_string(
        dir.path()
            .join("graphify-out")
            .join("stage_02_extract.json"),
    )
    .unwrap();
    assert!(raw.contains("crate:foo"), "crate node not merged: {raw}");
    assert!(raw.contains("crate_depends_on"), "crate edge not merged");
}

#[test]
fn extract_with_image_file_in_corpus_does_not_break_ast_run() {
    // Raster images join the corpus (#1110); the AST phase has no image
    // extractor and skips them, so an AST-only run (no backend) still succeeds.
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    fs::write(dir.path().join("diagram.png"), b"\x89PNG\r\n\x1a\nFAKE").unwrap();
    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();
    assert!(dir.path().join("graphify-out").join("graph.json").exists());
}

// Only meaningful when the `http` feature is absent: with it compiled in (e.g.
// the CI `--all-features` coverage run) `serve --transport http` takes the real
// transport path, so this negative assertion is gated off.
#[cfg(not(feature = "http"))]
#[test]
fn serve_http_without_feature_errors() {
    // The default binary build has no `http` feature, so `serve --transport http`
    // must fail loudly rather than silently fall back to stdio (#1143).
    let dir = tempfile::tempdir().unwrap();
    cli_no_backend()
        .current_dir(dir.path())
        .arg("serve")
        .arg("--transport")
        .arg("http")
        .assert()
        .failure()
        .stderr(contains("http").and(contains("feature")));
}

// Only meaningful when the `postgres` feature is absent: with it compiled in
// (e.g. the CI `--all-features` coverage run) `--postgres` tries to connect to a
// real database, so this negative assertion is gated off.
#[cfg(not(feature = "postgres"))]
#[test]
fn extract_postgres_without_feature_errors() {
    // The default binary build has no `postgres` feature, so `--postgres` must
    // fail loudly rather than silently ignore the flag.
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--postgres")
        .arg("postgresql://localhost/db")
        .arg("--no-cluster")
        .assert()
        .failure()
        .stderr(contains("postgres").and(contains("feature")));
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

/// The persisted manifest keys must be stored relative to the project root so
/// the file is portable across machines / checkout locations (#777), matching
/// Python's `_save_manifest(..., root=target)`. The absolute project path must
/// not leak into the keys.
#[test]
fn extract_manifest_keys_are_relative_to_root() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .assert()
        .success();

    let manifest =
        fs::read_to_string(dir.path().join("graphify-out").join("manifest.json")).unwrap();
    // Keys are stored posix-relative to the project root (#777): the relative
    // key is present and the absolute project path never leaks into the file.
    assert!(
        manifest.contains("\"src/main.py\""),
        "expected relative manifest key 'src/main.py': {manifest}"
    );
    let abs = dir.path().to_string_lossy().replace('\\', "/");
    assert!(
        !manifest.replace('\\', "/").contains(abs.as_str()),
        "absolute project path leaked into manifest: {manifest}"
    );
}

/// Mirrors `test_no_incremental_without_manifest`: a first extract with no
/// manifest must run a full scan, never the incremental path. Asserts the
/// specific incremental-mode phrases are absent (a bare "incremental" would also
/// match the temp path or unrelated wording).
#[test]
fn extract_without_manifest_is_full_scan() {
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    let assert = cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();
    let out = assert.get_output();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    assert!(!combined.contains("incremental update"), "{combined}");
    assert!(!combined.contains("incremental scan"), "{combined}");
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

// ── provider subcommand (#1084) ─────────────────────────────────────────────

#[test]
fn provider_add_list_show_remove_round_trip() {
    // `provider` writes to $HOME/.graphify/providers.json; point HOME at a temp
    // dir so the round-trip is isolated.
    let home = tempfile::tempdir().unwrap();

    cli()
        .env("HOME", home.path())
        .args([
            "provider",
            "add",
            "nvidia",
            "--base-url",
            "https://integrate.api.nvidia.com/v1",
            "--default-model",
            "minimaxai/minimax-m2.7",
            "--env-key",
            "NVIDIA_API_KEY",
        ])
        .assert()
        .success()
        .stdout(contains("Provider 'nvidia' added"));

    cli()
        .env("HOME", home.path())
        .args(["provider", "list"])
        .assert()
        .success()
        .stdout(contains("nvidia").and(contains("integrate.api.nvidia.com")));

    cli()
        .env("HOME", home.path())
        .args(["provider", "show", "nvidia"])
        .assert()
        .success()
        .stdout(contains("\"base_url\""));

    cli()
        .env("HOME", home.path())
        .args(["provider", "remove", "nvidia"])
        .assert()
        .success()
        .stdout(contains("Provider 'nvidia' removed"));

    cli()
        .env("HOME", home.path())
        .args(["provider", "list"])
        .assert()
        .success()
        .stdout(contains("No custom providers registered."));
}

#[test]
fn provider_add_rejects_builtin_name() {
    let home = tempfile::tempdir().unwrap();
    cli()
        .env("HOME", home.path())
        .args([
            "provider",
            "add",
            "claude",
            "--base-url",
            "http://x/v1",
            "--default-model",
            "m",
            "--env-key",
            "K",
        ])
        .assert()
        .failure()
        .stderr(contains("built-in provider"));
}

#[test]
fn provider_show_missing_fails() {
    let home = tempfile::tempdir().unwrap();
    cli()
        .env("HOME", home.path())
        .args(["provider", "show", "ghost"])
        .assert()
        .failure()
        .stderr(contains("not found"));
}

#[test]
fn provider_malformed_registry_is_not_clobbered() {
    // A present-but-malformed providers.json must abort (rather than be silently
    // overwritten), so the user's other providers aren't lost to a typo.
    let home = tempfile::tempdir().unwrap();
    let cfg = home.path().join(".graphify");
    fs::create_dir_all(&cfg).unwrap();
    fs::write(cfg.join("providers.json"), "{ this is not json").unwrap();
    cli()
        .env("HOME", home.path())
        .args([
            "provider",
            "add",
            "nvidia",
            "--base-url",
            "http://x/v1",
            "--default-model",
            "m",
            "--env-key",
            "K",
        ])
        .assert()
        .failure()
        .stderr(contains("malformed"));
}

#[test]
fn provider_add_rejects_non_finite_pricing() {
    // A non-finite price (`nan`/`inf`) would serialize to JSON `null` and read
    // back as the 0.0 default, silently losing the value, so it is rejected.
    let home = tempfile::tempdir().unwrap();
    cli()
        .env("HOME", home.path())
        .args([
            "provider",
            "add",
            "nvidia",
            "--base-url",
            "http://x/v1",
            "--default-model",
            "m",
            "--env-key",
            "K",
            "--pricing-input",
            "nan",
        ])
        .assert()
        .failure()
        .stderr(contains("finite"));
    // The rejected add must not have created a registry file.
    assert!(
        !home
            .path()
            .join(".graphify")
            .join("providers.json")
            .exists()
    );
}

// ── label command shares the cluster-only handler (#1097) ───────────────────

#[test]
fn label_no_backend_keeps_placeholders() {
    // With no backend configured, `label` degrades to `Community N` placeholders
    // and still regenerates the report/labels file.
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();

    cli_no_backend()
        .arg("label")
        .arg(dir.path())
        .arg("--no-viz")
        .assert()
        .success();

    let labels = fs::read_to_string(
        dir.path()
            .join("graphify-out")
            .join(".graphify_labels.json"),
    )
    .unwrap();
    assert!(
        labels.contains("Community"),
        "expected placeholder labels: {labels}"
    );
}

#[test]
fn label_accepts_model_flag() {
    // `label --model` parses and threads through to the labeling path (#b304331).
    // With no backend key the run still degrades to placeholders, proving the
    // flag is accepted end-to-end without error.
    let dir = tempfile::tempdir().unwrap();
    write_python_project(dir.path());
    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();

    cli_no_backend()
        .arg("label")
        .arg(dir.path())
        .arg("--backend")
        .arg("gemini")
        .arg("--model")
        .arg("gemini-3.1-flash-lite")
        .arg("--no-viz")
        .assert()
        .success();

    let labels = fs::read_to_string(
        dir.path()
            .join("graphify-out")
            .join(".graphify_labels.json"),
    )
    .unwrap();
    assert!(
        labels.contains("Community"),
        "expected placeholder labels: {labels}"
    );
}

/// #1347/#1350: a no-op incremental `extract --no-cluster` re-run must leave
/// graph.json byte-identical. The first run persists `manifest.json` (parity with
/// graphify-py `__main__.py:4492`), so the second run takes the incremental path;
/// it rebuilds deterministically from the sorted changed+unchanged union via
/// `build_from_json`, and `to_json` carries no timestamp, so the output stays
/// byte-identical without an explicit no-op short-circuit.
#[test]
fn extract_no_cluster_incremental_noop_preserves_existing_graph() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("app.py"), "def alpha():\n    return 1\n").unwrap();

    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();
    let graph_path = dir.path().join("graphify-out").join("graph.json");
    assert!(
        dir.path()
            .join("graphify-out")
            .join("manifest.json")
            .exists(),
        "--no-cluster must persist manifest.json so a re-run takes the incremental path"
    );
    let before = fs::read_to_string(&graph_path).unwrap();
    let before_json: serde_json::Value = serde_json::from_str(&before).unwrap();
    assert!(
        before_json
            .get("nodes")
            .and_then(|n| n.as_array())
            .is_some_and(|a| !a.is_empty()),
        "first run should produce a non-empty code graph"
    );

    cli_no_backend()
        .arg("extract")
        .arg(dir.path())
        .arg("--no-cluster")
        .assert()
        .success();
    let after = fs::read_to_string(&graph_path).unwrap();
    let after_json: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert!(
        after_json
            .get("nodes")
            .and_then(|n| n.as_array())
            .is_some_and(|a| !a.is_empty()),
        "no-op incremental run must not empty the graph"
    );
    assert_eq!(after, before, "no-op incremental run changed graph.json");
}
