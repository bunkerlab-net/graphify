//! Coverage tests for `graphify export` subcommands and related commands.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;

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

// ── export falkordb (cypher.txt, no --push) ─────────────────────────────────

#[test]
fn export_falkordb_writes_cypher_without_push() {
    // Without --push, `export falkordb` writes OpenCypher to cypher.txt (the
    // live redis push is feature-gated and needs a server). #1175.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    write_graph_json(&graph_path);

    cli()
        .arg("export")
        .arg("falkordb")
        .arg("--graph")
        .arg(&graph_path)
        .assert()
        .success()
        .stdout(contains("cypher.txt"));

    let cypher = fs::read_to_string(out.join("cypher.txt")).unwrap();
    assert!(
        cypher.contains("MERGE"),
        "expected MERGE statements: {cypher}"
    );
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

// ── cluster-only remaps labels to previous cids (#1027) ──────────────────────

#[test]
fn cluster_only_remaps_labels_to_previous_cids() {
    // cluster-only must invoke remap_communities_to_previous so an existing
    // .graphify_labels.json keeps tracking the same conceptual community after
    // re-clustering. Without the remap, the partitioner renumbers communities
    // to 0,1,... and the prior sentinel-keyed labels are orphaned (#1027).
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    let labels_path = out.join(".graphify_labels.json");

    // Two disconnected pairs → 2 communities. Tag the first pair with sentinel
    // 4242 and the second with 9999, then key the labels file on those ids.
    let (sentinel_a, sentinel_b) = (4242, 9999);
    fs::write(
        &graph_path,
        format!(
            r#"{{"nodes":[
                {{"id":"a","label":"A","file_type":"code","source_file":"a.py","community":{sentinel_a}}},
                {{"id":"b","label":"B","file_type":"code","source_file":"b.py","community":{sentinel_a}}},
                {{"id":"c","label":"C","file_type":"code","source_file":"c.py","community":{sentinel_b}}},
                {{"id":"d","label":"D","file_type":"code","source_file":"d.py","community":{sentinel_b}}}
            ],"edges":[
                {{"source":"a","target":"b","context":"CALLS","confidence":"EXTRACTED","source_file":"a.py"}},
                {{"source":"c","target":"d","context":"CALLS","confidence":"EXTRACTED","source_file":"c.py"}}
            ]}}"#
        ),
    )
    .unwrap();
    fs::write(
        &labels_path,
        format!(r#"{{"{sentinel_a}":"First Group","{sentinel_b}":"Second Group"}}"#),
    )
    .unwrap();

    cli()
        .arg("cluster-only")
        .arg(dir.path())
        .arg("--no-viz")
        .assert()
        .success();

    let final_graph: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&graph_path).unwrap()).unwrap();
    let final_labels: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&labels_path).unwrap()).unwrap();

    let actual_cids: std::collections::HashSet<i64> = final_graph
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .unwrap()
        .iter()
        .filter_map(|n| n.get("community").and_then(serde_json::Value::as_i64))
        .collect();
    let label_cids: std::collections::HashSet<i64> = final_labels
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|k| k.parse::<i64>().ok())
        .collect();

    // Non-empty intersection mirrors graphify-py's `assert overlap`: the remap
    // guarantees at least one prior cid survives. Asserting both survive would
    // over-constrain beyond the reference test and couple to partitioner output.
    assert!(
        actual_cids.intersection(&label_cids).next().is_some(),
        "after cluster-only, at least one prior label cid must still appear in \
         graph.json community attrs. actual={actual_cids:?} labels={label_cids:?}"
    );
}

/// Helper: write a minimal two-node connected graph so `cluster-only` resolves
/// exactly one community.
fn write_min_graph(graph_path: &Path) {
    fs::write(
        graph_path,
        r#"{"nodes":[
            {"id":"a","label":"A","file_type":"code","source_file":"a.py"},
            {"id":"b","label":"B","file_type":"code","source_file":"b.py"}
        ],"edges":[
            {"source":"a","target":"b","context":"CALLS","confidence":"EXTRACTED","source_file":"a.py"}
        ]}"#,
    )
    .unwrap();
}

#[test]
fn cluster_only_no_label_preserves_existing_labels() {
    // `--no-label` must NOT wipe a curated labels file to placeholders. An
    // existing file already means no LLM call, so `--no-label` is a harmless
    // no-op here: the curated names are preserved, not clobbered.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    let labels_path = out.join(".graphify_labels.json");
    write_min_graph(&graph_path);
    fs::write(&labels_path, r#"{"0":"Curated Name"}"#).unwrap();

    cli()
        .arg("cluster-only")
        .arg(dir.path())
        .arg("--no-label")
        .arg("--no-viz")
        .assert()
        .success();

    let labels = fs::read_to_string(&labels_path).unwrap();
    assert!(
        labels.contains("Curated Name"),
        "--no-label must preserve curated names, not wipe them: {labels}"
    );
}

#[test]
fn cluster_only_no_label_placeholders_when_no_file() {
    // With no existing labels file, `--no-label` produces `Community N`
    // placeholders (and no LLM call).
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    let labels_path = out.join(".graphify_labels.json");
    write_min_graph(&graph_path);

    cli()
        .arg("cluster-only")
        .arg(dir.path())
        .arg("--no-label")
        .arg("--no-viz")
        .assert()
        .success();

    let labels = fs::read_to_string(&labels_path).unwrap();
    assert!(
        labels.contains("Community "),
        "--no-label with no file must write `Community N` placeholders: {labels}"
    );
}

#[test]
fn cluster_only_does_not_clobber_malformed_labels() {
    // A malformed `.graphify_labels.json` must be preserved (not silently
    // overwritten with placeholders) so hand-curated edits aren't lost.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    let labels_path = out.join(".graphify_labels.json");
    write_min_graph(&graph_path);
    let malformed = "{ this is : not valid json ";
    fs::write(&labels_path, malformed).unwrap();

    cli()
        .arg("cluster-only")
        .arg(dir.path())
        .arg("--no-viz")
        .assert()
        .success()
        .stderr(contains("leaving the existing file untouched"));

    // The malformed file must be left exactly as written.
    let after = fs::read_to_string(&labels_path).unwrap();
    assert_eq!(
        after, malformed,
        "malformed labels file must not be clobbered"
    );
}

#[test]
fn cluster_only_writes_community_name_from_labels_file() {
    // Path-1 (labels file exists, no --force, no LLM call): cluster-only must
    // stamp each node's `community_name` in the rewritten graph.json from the
    // resolved labels, mirroring Python `__main__.py:3283`
    // `to_json(G, communities, ..., community_labels=labels)`.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("graphify-out");
    fs::create_dir_all(&out).unwrap();
    let graph_path = out.join("graph.json");
    let labels_path = out.join(".graphify_labels.json");

    // Tag both nodes with a sentinel community so the remap pins the resolved
    // community id to 4242, keeping the preset label attached after re-cluster.
    fs::write(
        &graph_path,
        r#"{"nodes":[
            {"id":"a","label":"A","file_type":"code","source_file":"a.py","community":4242},
            {"id":"b","label":"B","file_type":"code","source_file":"b.py","community":4242}
        ],"edges":[
            {"source":"a","target":"b","context":"CALLS","confidence":"EXTRACTED","source_file":"a.py"}
        ]}"#,
    )
    .unwrap();
    fs::write(&labels_path, r#"{"4242":"Auth Layer"}"#).unwrap();

    cli()
        .arg("cluster-only")
        .arg(dir.path())
        .arg("--no-viz")
        .assert()
        .success();

    let final_graph: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&graph_path).unwrap()).unwrap();
    let nodes = final_graph
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    assert!(
        nodes.iter().any(
            |n| n.get("community_name").and_then(serde_json::Value::as_str) == Some("Auth Layer")
        ),
        "cluster-only must stamp community_name from the labels file onto \
         graph.json nodes: {final_graph}"
    );
}

// ── #1789: graph.json node ids are portable across checkout paths ─────────────

/// The committed `graph.json`'s node ids must be relative to the scan root —
/// never embedding the absolute path — so the same repo yields identical ids on
/// any machine/checkout and leaks no local username/home. Extracts the same
/// corpus from two different absolute prefixes and asserts the id sets match and
/// carry no path component.
#[test]
fn graph_json_node_ids_are_portable_across_checkout_paths() {
    fn build(root: &Path) -> Vec<String> {
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg").join("mod.py"), "def f(): return 1\n").unwrap();
        fs::write(
            root.join("pkg").join("app.py"),
            "from pkg.mod import f\ndef g(): return f()\n",
        )
        .unwrap();
        cli()
            .current_dir(root)
            .arg("extract")
            .arg(".")
            .arg("--code-only")
            .arg("--no-cluster")
            .assert()
            .success();
        let bytes = fs::read(root.join("graphify-out").join("graph.json")).unwrap();
        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let mut ids: Vec<String> = data["nodes"]
            .as_array()
            .expect("nodes array")
            .iter()
            .filter_map(|n| {
                n.get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        ids.sort();
        ids
    }

    let tmp = tempfile::tempdir().unwrap();
    let a = build(&tmp.path().join("alice_home").join("proj"));
    let b = build(
        &tmp.path()
            .join("bob_elsewhere")
            .join("checkout")
            .join("proj"),
    );
    assert_eq!(
        a, b,
        "node ids differ across checkout paths: {a:?} vs {b:?}"
    );
    assert!(!a.is_empty(), "extraction produced no nodes");
    // No id segment may be an absolute-path component (username/home leak).
    let leak = [
        "alice_home",
        "bob_elsewhere",
        "checkout",
        "tmp",
        "private",
        "users",
        "home",
        "var",
    ];
    for ident in &a {
        for part in ident.split('_') {
            assert!(
                !leak.contains(&part),
                "node id embeds an absolute-path component: {a:?}"
            );
        }
    }
}
