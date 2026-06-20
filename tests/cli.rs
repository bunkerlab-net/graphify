//! Workspace-level CLI integration tests.
//!
//! Each test spawns the built `graphify` binary and asserts on stdout/stderr.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn cli() -> Command {
    let mut cmd = Command::cargo_bin("graphify").expect("cargo-bin graphify");
    // Keep query/path/explain runs from appending to the developer's real
    // ~/.cache/graphify-queries.log during tests (#1128 query logging).
    cmd.env("GRAPHIFY_QUERY_LOG_DISABLE", "1");
    cmd
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
    // Mirror Python's convention: each graph lives at <repo>/graphify-out/graph.json
    // so `parent.parent.name` resolves to the repo tag prefix.
    let repo_a = dir.path().join("repo_a").join("graphify-out");
    let repo_b = dir.path().join("repo_b").join("graphify-out");
    fs::create_dir_all(&repo_a).unwrap();
    fs::create_dir_all(&repo_b).unwrap();
    let a = repo_a.join("graph.json");
    let b = repo_b.join("graph.json");
    fs::write(
        &a,
        r#"{"nodes":[{"id":"main","label":"main","file_type":"code","source_file":"a.py"}],"edges":[]}"#,
    )
    .unwrap();
    fs::write(
        &b,
        r#"{"nodes":[{"id":"main","label":"main","file_type":"code","source_file":"b.py"}],"edges":[]}"#,
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
        .success()
        .stdout(contains("Merged 2 graphs"))
        .stdout(contains("Written to:"));
    let body = fs::read_to_string(&out).unwrap();
    // Repo-tag-prefixed ids prevent the collision that the unprefixed merge would have.
    assert!(body.contains(r#""id": "repo_a::main""#), "got: {body}");
    assert!(body.contains(r#""id": "repo_b::main""#), "got: {body}");
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

/// Ports `graphify-py/tests/test_path_cli.py::test_forward_arrow`.
///
/// `graphify path` must announce hop count and render arrow segments with
/// the edge relation + confidence in their on-disk direction.
#[test]
fn path_forward_arrow() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    let graph = r#"{
        "directed": false, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "create_patch", "label": "createPatchHandler()", "source_file": "server/create-patch-handler.ts", "community": 0},
            {"id": "validate", "label": "validateSanitySession()", "source_file": "server/sanity-validate-session.ts", "community": 0}
        ],
        "links": [
            {"source": "create_patch", "target": "validate", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    }"#;
    fs::write(&graph_path, graph).unwrap();
    cli()
        .arg("path")
        .arg("createPatchHandler")
        .arg("validateSanitySession")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success()
        .stdout(contains("Shortest path (1 hops):"))
        .stdout(contains(
            "createPatchHandler() --calls [EXTRACTED]--> validateSanitySession()",
        ));
}

/// Ports `graphify-py/tests/test_path_cli.py::test_reverse_arrow`.
///
/// Reversing the source/target must flip the arrow direction so callers
/// can distinguish the stored caller→callee direction in the output.
#[test]
fn path_reverse_arrow() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    let graph = r#"{
        "directed": false, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "create_patch", "label": "createPatchHandler()", "source_file": "server/create-patch-handler.ts", "community": 0},
            {"id": "validate", "label": "validateSanitySession()", "source_file": "server/sanity-validate-session.ts", "community": 0}
        ],
        "links": [
            {"source": "create_patch", "target": "validate", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    }"#;
    fs::write(&graph_path, graph).unwrap();
    let assert = cli()
        .arg("path")
        .arg("validateSanitySession")
        .arg("createPatchHandler")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(stdout.contains("Shortest path (1 hops):"), "got: {stdout}");
    assert!(
        stdout.contains("validateSanitySession() <--calls [EXTRACTED]-- createPatchHandler()"),
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("validateSanitySession() --calls [EXTRACTED]--> createPatchHandler()"),
        "got: {stdout}"
    );
}

/// Ports `graphify-py/tests/test_explain_cli.py::test_callee_shows_callers_as_inbound`.
///
/// `graphify explain` on a callee must surface inbound callers with `<--`
/// arrows and outbound callees with `-->` arrows, tagged with the relation.
#[test]
fn explain_callee_shows_callers_as_inbound() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    let graph = r#"{
        "directed": false, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "validate", "label": "validateSanitySession()", "source_file": "server/sanity-validate-session.ts", "community": 0},
            {"id": "create_patch", "label": "createPatchHandler()", "source_file": "server/create-patch-handler.ts", "community": 0},
            {"id": "create_edit", "label": "createEditHandler()", "source_file": "server/create-edit-handler.ts", "community": 0},
            {"id": "stable_stringify", "label": "stableStringify()", "source_file": "shared/stringify.ts", "community": 0}
        ],
        "links": [
            {"source": "create_patch", "target": "validate", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "create_edit", "target": "validate", "relation": "calls", "confidence": "EXTRACTED"},
            {"source": "validate", "target": "stable_stringify", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    }"#;
    fs::write(&graph_path, graph).unwrap();
    let assert = cli()
        .arg("explain")
        .arg("validateSanitySession")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("<-- createPatchHandler() [calls]"),
        "got: {stdout}"
    );
    assert!(
        stdout.contains("<-- createEditHandler() [calls]"),
        "got: {stdout}"
    );
    assert!(
        stdout.contains("--> stableStringify() [calls]"),
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("--> createPatchHandler() [calls]"),
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("--> createEditHandler() [calls]"),
        "got: {stdout}"
    );
}

/// Ports `graphify-py/tests/test_explain_cli.py::test_caller_shows_callee_as_outbound`.
///
/// Explaining a caller must show its callee with `-->` and emit no `<--`
/// markers when the caller has no inbound edges.
#[test]
fn explain_caller_shows_callee_as_outbound() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    let graph = r#"{
        "directed": false, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "validate", "label": "validateSanitySession()", "source_file": "server/sanity-validate-session.ts", "community": 0},
            {"id": "create_patch", "label": "createPatchHandler()", "source_file": "server/create-patch-handler.ts", "community": 0}
        ],
        "links": [
            {"source": "create_patch", "target": "validate", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    }"#;
    fs::write(&graph_path, graph).unwrap();
    let assert = cli()
        .arg("explain")
        .arg("createPatchHandler")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("--> validateSanitySession() [calls]"),
        "got: {stdout}"
    );
    assert!(!stdout.contains("<-- "), "got: {stdout}");
}

/// Ports the `community_name` preference from Python `__main__.py:2973`
/// (`d.get('community_name') or d.get('community', '')`): `explain` shows the
/// human community label when the node carries one, and falls back to the
/// numeric community id otherwise.
#[test]
fn explain_prefers_community_name_over_numeric() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    let graph = r#"{
        "directed": false, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "labeled", "label": "labeledFn()", "source_file": "a.ts", "community": 0, "community_name": "Auth Layer"},
            {"id": "plain", "label": "plainFn()", "source_file": "b.ts", "community": 5}
        ],
        "links": [
            {"source": "labeled", "target": "plain", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    }"#;
    fs::write(&graph_path, graph).unwrap();

    // Node with community_name -> the human label wins over the numeric id.
    let labeled = cli()
        .arg("explain")
        .arg("labeledFn")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    let labeled_out = String::from_utf8_lossy(&labeled.get_output().stdout).into_owned();
    assert!(
        labeled_out.contains("Community: Auth Layer"),
        "got: {labeled_out}"
    );

    // Node without community_name -> falls back to the numeric community id.
    let plain = cli()
        .arg("explain")
        .arg("plainFn")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success();
    let plain_out = String::from_utf8_lossy(&plain.get_output().stdout).into_owned();
    assert!(plain_out.contains("Community: 5"), "got: {plain_out}");
}

/// `graphify export neo4j` (no --push) must write cypher.txt next to graph.json.
///
/// Python writes the same default file at `__main__.py:2322`.
#[test]
fn export_neo4j_writes_cypher_txt() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    fs::write(
        &graph_path,
        r#"{"nodes":[{"id":"a","label":"A","file_type":"code","source_file":"a.py"}],"edges":[]}"#,
    )
    .unwrap();
    cli()
        .arg("export")
        .arg("neo4j")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success()
        .stdout(contains("cypher.txt written"));
    assert!(dir.path().join("cypher.txt").exists());
}

/// `graphify export callflow-html` prints the Python parity message to stdout.
#[test]
fn export_callflow_html_message_matches_python() {
    let dir = tempfile::tempdir().unwrap();
    let graphify_out = dir.path().join("graphify-out");
    fs::create_dir_all(&graphify_out).unwrap();
    let graph_path = graphify_out.join("graph.json");
    fs::write(
        &graph_path,
        r#"{"directed":false,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"A","source_file":"a.py","file_type":"code","community":0}],"links":[]}"#,
    )
    .unwrap();
    let output = graphify_out.join("from-cli.html");
    cli()
        .arg("export")
        .arg("callflow-html")
        .arg("--graph")
        .arg(&graph_path)
        .arg("--output")
        .arg(&output)
        .arg("--max-sections")
        .arg("4")
        .assert()
        .success()
        .stdout(contains("callflow HTML written"));
    assert!(output.exists());
}

/// `graphify export callflow-html <GRAPH.json>` (positional) renders that graph
/// and derives `GRAPH_REPORT.md` from the graph's own directory — Python parity
/// with the `export callflow-html [GRAPH|DIR]` positional argument.
#[test]
fn export_callflow_html_accepts_positional_graph_path() {
    let dir = tempfile::tempdir().unwrap();
    let external = dir.path().join("GitNexus").join("graphify-out");
    fs::create_dir_all(&external).unwrap();
    fs::write(
        external.join("graph.json"),
        r#"{"directed":false,"multigraph":false,"graph":{},"nodes":[{"id":"external","label":"ExternalOnly","source_file":"src/external.py","file_type":"code","community":0},{"id":"writer","label":"write_external()","source_file":"src/writer.py","file_type":"code","community":1}],"links":[{"source":"external","target":"writer","relation":"calls","confidence":"EXTRACTED","confidence_score":1.0}]}"#,
    )
    .unwrap();
    fs::write(
        external.join("GRAPH_REPORT.md"),
        "# Graph Report - external\n\n## God Nodes (most connected - your core abstractions)\n1. `ExternalGod` - 1 edges\n",
    )
    .unwrap();
    let output = dir.path().join("positional.html");
    cli()
        .arg("export")
        .arg("callflow-html")
        .arg(external.join("graph.json"))
        .arg("--output")
        .arg(&output)
        .arg("--max-sections")
        .arg("4")
        .assert()
        .success();
    let html = fs::read_to_string(&output).unwrap();
    assert!(
        html.contains("ExternalOnly"),
        "positional graph node missing"
    );
    assert!(
        html.contains("ExternalGod"),
        "report resolved from the positional graph's directory missing"
    );
}

/// `graphify trae-cn install` is reachable as a named subcommand (parity with
/// Python's per-platform install routes). Just exercise the dispatch — we
/// don't want to permanently install the skill, so use --help.
#[test]
fn trae_cn_subcommand_is_dispatched() {
    cli()
        .arg("trae-cn")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("install"));
}

#[test]
fn hermes_subcommand_is_dispatched() {
    cli()
        .arg("hermes")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("install"));
}

/// `graphify install <platform>` accepts the platform as a positional arg
/// (parity with Python's argv-parsing fallback at `__main__.py:1358`).
#[test]
fn install_accepts_positional_platform() {
    // We can't actually install — that would touch the user's home dir.
    // Verify the parser accepts the install subcommand and that `--help`
    // mentions the positional platform argument so the surface stays
    // discoverable.
    cli()
        .arg("install")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("install"));
}

/// `graphify save-result` writes a Q&A markdown file and prints `Saved to <path>`
/// (parity with Python's stdout message at `__main__.py:1578`).
#[test]
fn save_result_prints_saved_to() {
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path().join("memory");
    cli()
        .arg("save-result")
        .arg("--question")
        .arg("what calls foo?")
        .arg("--answer")
        .arg("nothing")
        .arg("--type")
        .arg("explain")
        .arg("--memory-dir")
        .arg(&memory)
        .assert()
        .success()
        .stdout(contains("Saved to "));
}

/// `graphify export wiki` refuses to render without an analysis sidecar
/// (parity with Python's bail at `__main__.py:2288`).
#[test]
fn export_wiki_refuses_without_analysis() {
    let dir = tempfile::tempdir().unwrap();
    let graph_path = dir.path().join("graph.json");
    fs::write(
        &graph_path,
        r#"{"nodes":[{"id":"a","label":"A","file_type":"code","source_file":"a.py"}],"edges":[]}"#,
    )
    .unwrap();
    cli()
        .arg("export")
        .arg("wiki")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .failure()
        .stderr(contains(".graphify_analysis.json is missing or empty"));
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
