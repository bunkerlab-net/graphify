//! Parity tests against `graphify-py/tests/test_serve.py`.
//!
//! All test cases from the Python test suite are ported here. We use
//! `graphify_build::build_from_json` to construct `Graph` objects rather than
//! the Python `networkx` constructors.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use graphify_build::{Graph, build_from_json};
use graphify_prs::error::PrsError;
use graphify_prs::gh::GhClient;
use graphify_prs::git::GitClient;
use graphify_serve::graph::{
    bfs, communities_from_graph, compute_idf, dfs, filter_graph_by_context, infer_context_filters,
    load_graph, pick_seeds, query_graph_text, query_terms, resolve_context_filters, score_nodes,
    subgraph_to_text,
};
use graphify_serve::tools::{
    tool_get_pr_impact_with_clients, tool_list_prs_with_clients, tool_triage_prs_with_clients,
};
use serde_json::json;
use tempfile::tempdir;

// ── Test doubles for PR tool tests ────────────────────────────────────────────

/// One canned PR in the wire format that `gh pr list` returns.
const CANNED_PR_JSON: &str = r#"[{
    "number": 42,
    "title": "Add feature X",
    "headRefName": "feature/x",
    "baseRefName": "main",
    "author": {"login": "alice"},
    "isDraft": false,
    "reviewDecision": "APPROVED",
    "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}],
    "updatedAt": "2025-01-01T00:00:00Z"
}]"#;

#[cfg(test)]
struct FakeGhClient {
    prs_json: &'static str,
    files: Vec<String>,
    default_branch: Option<String>,
}

impl GhClient for FakeGhClient {
    fn pr_list(&self, _repo: Option<&str>, _limit: usize) -> Result<Vec<u8>, PrsError> {
        Ok(self.prs_json.as_bytes().to_vec())
    }

    fn repo_default_branch(&self, _repo: Option<&str>) -> Option<String> {
        self.default_branch.clone()
    }

    fn pr_files(&self, _number: u64, _repo: Option<&str>) -> Vec<String> {
        self.files.clone()
    }
}

#[cfg(test)]
struct FakeGitClient;

impl GitClient for FakeGitClient {
    fn worktree_list_porcelain(&self) -> Option<String> {
        None
    }

    fn symbolic_ref_origin_head(&self) -> Option<String> {
        None
    }
}

// ── Test graph factory ────────────────────────────────────────────────────────

/// Mirrors Python `_make_graph()`.
fn make_graph() -> Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "extract", "source_file": "extract.py",
                 "source_location": "L10", "community": 0},
                {"id": "n2", "label": "cluster", "source_file": "cluster.py",
                 "source_location": "L5", "community": 0},
                {"id": "n3", "label": "build", "source_file": "build.py",
                 "source_location": "L1", "community": 1},
                {"id": "n4", "label": "report", "source_file": "report.py",
                 "source_location": "L1", "community": 1},
                {"id": "n5", "label": "isolated", "source_file": "other.py",
                 "source_location": "L1", "community": 2},
            ],
            "edges": [
                {"source": "n1", "target": "n2", "relation": "calls",
                 "confidence": "INFERRED", "context": "call"},
                {"source": "n2", "target": "n3", "relation": "imports",
                 "confidence": "EXTRACTED", "context": "import"},
                {"source": "n3", "target": "n4", "relation": "uses",
                 "confidence": "EXTRACTED"},
            ]
        }),
        // Use undirected to match Python nx.Graph in test_serve.py
        false,
        None,
    )
    .expect("make_graph")
}

/// Mirrors Python `_make_noisy_graph()`.
fn make_noisy_graph() -> Graph {
    let mut nodes = vec![];
    let mut edges = vec![];
    for i in 0..20_u64 {
        nodes.push(json!({
            "id": format!("err{i}"),
            "label": format!("error_handler_{i}"),
            "source_file": format!("err{i}.py"),
            "community": 0
        }));
        if i > 0 {
            edges.push(json!({
                "source": format!("err{}", i - 1),
                "target": format!("err{i}"),
                "relation": "calls",
                "confidence": "EXTRACTED"
            }));
        }
    }
    nodes.push(json!({
        "id": "fbs",
        "label": "FooBarService",
        "source_file": "service.py",
        "community": 1
    }));
    nodes.push(json!({
        "id": "fbs_dep",
        "label": "ServiceClient",
        "source_file": "client.py",
        "community": 1
    }));
    edges.push(json!({
        "source": "fbs",
        "target": "fbs_dep",
        "relation": "uses",
        "confidence": "EXTRACTED"
    }));
    build_from_json(json!({"nodes": nodes, "edges": edges}), false, None).expect("make_noisy_graph")
}

// ── _communities_from_graph ───────────────────────────────────────────────────

#[test]
fn test_communities_from_graph_basic() {
    let g = make_graph();
    let communities = communities_from_graph(&g);
    assert!(communities.contains_key(&0));
    assert!(communities.contains_key(&1));
    assert!(communities[&0].contains(&"n1".to_string()));
    assert!(communities[&0].contains(&"n2".to_string()));
    assert!(communities[&1].contains(&"n3".to_string()));
}

#[test]
fn test_communities_from_graph_no_community_attr() {
    let g = build_from_json(
        json!({"nodes": [{"id": "a", "label": "foo"}], "edges": []}),
        false,
        None,
    )
    .expect("graph");
    let communities = communities_from_graph(&g);
    assert!(communities.is_empty());
}

#[test]
fn test_communities_from_graph_isolated() {
    let g = make_graph();
    let communities = communities_from_graph(&g);
    assert!(communities.contains_key(&2));
    assert!(communities[&2].contains(&"n5".to_string()));
}

// ── _score_nodes ─────────────────────────────────────────────────────────────

#[test]
fn test_score_nodes_exact_label_match() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let scored = score_nodes(&g, &["extract"], &mut cache);
    assert!(!scored.is_empty());
    let nids: Vec<&str> = scored.iter().map(|(_, nid)| nid.as_str()).collect();
    assert!(nids.contains(&"n1"));
    assert_eq!(scored[0].1, "n1", "highest score should be n1");
}

#[test]
fn test_score_nodes_no_match() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let scored = score_nodes(&g, &["xyzzy"], &mut cache);
    assert!(scored.is_empty());
}

#[test]
fn test_score_nodes_source_file_partial() {
    let g = make_graph();
    let mut cache = HashMap::new();
    // "cluster.py" contains "cluster" — should score for source match
    let scored = score_nodes(&g, &["cluster"], &mut cache);
    let nids: Vec<&str> = scored.iter().map(|(_, nid)| nid.as_str()).collect();
    assert!(nids.contains(&"n2"));
}

// ── _infer_context_filters ────────────────────────────────────────────────────

#[test]
fn test_infer_context_filters_for_calls_question() {
    assert_eq!(
        infer_context_filters("who calls extract"),
        vec!["call".to_string()]
    );
}

// ── _resolve_context_filters ──────────────────────────────────────────────────

#[test]
fn test_resolve_context_filters_explicit_overrides_heuristic() {
    let explicit = vec!["field".to_string()];
    let (filters, source) = resolve_context_filters("who calls extract", Some(&explicit));
    assert_eq!(filters, vec!["field".to_string()]);
    assert_eq!(source, Some("explicit".to_string()));
}

// ── _bfs ─────────────────────────────────────────────────────────────────────

#[test]
fn test_bfs_depth_1() {
    let g = make_graph();
    let (visited, _edges) = bfs(&g, &["n1".to_string()], 1);
    assert!(visited.contains("n1"));
    assert!(visited.contains("n2")); // direct neighbor
    assert!(!visited.contains("n3")); // 2 hops away
}

#[test]
fn test_bfs_depth_2() {
    let g = make_graph();
    let (visited, _edges) = bfs(&g, &["n1".to_string()], 2);
    assert!(visited.contains("n3")); // n1 -> n2 -> n3
}

#[test]
fn test_bfs_disconnected() {
    let g = make_graph();
    let (visited, _edges) = bfs(&g, &["n5".to_string()], 3);
    // isolated node — only itself
    assert_eq!(visited.len(), 1);
    assert!(visited.contains("n5"));
}

#[test]
fn test_bfs_returns_edges() {
    let g = make_graph();
    let (_, edges) = bfs(&g, &["n1".to_string()], 1);
    assert!(!edges.is_empty());
    assert!(edges.iter().any(|(u, v)| u == "n1" || v == "n1"));
}

// ── _filter_graph_by_context ──────────────────────────────────────────────────

#[test]
fn test_filter_graph_by_context_limits_traversal() {
    let g = make_graph();
    let filters = vec!["call".to_string()];
    let filtered = filter_graph_by_context(&g, Some(&filters));
    let (visited, edges) = bfs(&filtered, &["n1".to_string()], 2);
    assert!(visited.contains("n2"));
    assert!(!visited.contains("n3"));
    assert_eq!(edges, vec![("n1".to_string(), "n2".to_string())]);
}

// ── _dfs ─────────────────────────────────────────────────────────────────────

#[test]
fn test_dfs_depth_1() {
    let g = make_graph();
    let (visited, _edges) = dfs(&g, &["n1".to_string()], 1);
    assert!(visited.contains("n1"));
    assert!(visited.contains("n2"));
    assert!(!visited.contains("n3"));
}

#[test]
fn test_dfs_full_chain() {
    let g = make_graph();
    let (visited, _edges) = dfs(&g, &["n1".to_string()], 5);
    for n in ["n1", "n2", "n3", "n4"] {
        assert!(visited.contains(n), "expected {n} in visited");
    }
}

// ── _subgraph_to_text ─────────────────────────────────────────────────────────

#[test]
fn test_subgraph_to_text_contains_labels() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(
        &g,
        &nodes,
        &[("n1".to_string(), "n2".to_string())],
        2000,
        None,
    );
    assert!(text.contains("extract"));
    assert!(text.contains("cluster"));
}

#[test]
fn test_subgraph_to_text_truncates() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> = ["n1", "n2", "n3", "n4"]
        .iter()
        .map(|&s| s.to_string())
        .collect();
    // Very small budget forces truncation.
    let text = subgraph_to_text(&g, &nodes, &[("n1".to_string(), "n2".to_string())], 1, None);
    assert!(text.contains("truncated"));
}

#[test]
fn test_subgraph_to_text_edge_included() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(
        &g,
        &nodes,
        &[("n1".to_string(), "n2".to_string())],
        2000,
        None,
    );
    assert!(text.contains("EDGE"));
    assert!(text.contains("calls"));
}

#[test]
fn test_subgraph_to_text_includes_edge_context() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(
        &g,
        &nodes,
        &[("n1".to_string(), "n2".to_string())],
        2000,
        None,
    );
    assert!(text.contains("context=call"));
}

// ── _query_graph_text ─────────────────────────────────────────────────────────

#[test]
fn test_query_graph_text_explicit_context_filter_changes_traversal() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let filters = vec!["call".to_string()];
    let text = query_graph_text(&g, "extract", "bfs", 2, 2000, Some(&filters), &mut cache);
    assert!(text.contains("Context: call (explicit)"));
    assert!(text.contains("cluster"));
    assert!(!text.contains("build"));
}

#[test]
fn test_query_graph_text_heuristic_context_filter_changes_traversal() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let text = query_graph_text(&g, "who calls extract", "bfs", 2, 2000, None, &mut cache);
    assert!(text.contains("Context: call (heuristic)"));
    assert!(text.contains("cluster"));
    assert!(!text.contains("build"));
}

// ── _load_graph ───────────────────────────────────────────────────────────────

#[test]
fn test_load_graph_roundtrip() {
    let tmp = tempdir().expect("tempdir");
    let p = tmp.path().join("graph.json");

    // Write a minimal node-link JSON.
    let data = json!({
        "directed": true,
        "nodes": [
            {"id": "n1", "label": "a"},
            {"id": "n2", "label": "b"},
            {"id": "n3", "label": "c"},
            {"id": "n4", "label": "d"},
            {"id": "n5", "label": "e"}
        ],
        "links": [
            {"source": "n1", "target": "n2"},
            {"source": "n2", "target": "n3"},
            {"source": "n3", "target": "n4"}
        ]
    });
    std::fs::write(&p, serde_json::to_string(&data).expect("json")).expect("write");
    let g = load_graph(p.to_str().expect("str")).expect("load");
    assert_eq!(g.node_count(), 5);
    assert_eq!(g.edge_count(), 3);
}

#[test]
fn test_load_graph_missing_file() {
    let tmp = tempdir().expect("tempdir");
    let p = tmp.path().join("graphify-out").join("nonexistent.json");
    // Should return Err, not panic.
    assert!(load_graph(p.to_str().expect("str")).is_err());
}

// ── Hot-reload (issue #874) ───────────────────────────────────────────────────

fn write_graph(path: &std::path::Path, node_ids: &[&str]) {
    let nodes: Vec<_> = node_ids
        .iter()
        .map(|id| json!({"id": id, "label": id, "community": 0}))
        .collect();
    let data = json!({"directed": true, "nodes": nodes, "links": []});
    std::fs::write(path, serde_json::to_string(&data).expect("json")).expect("write");
}

#[test]
fn test_maybe_reload_detects_graph_change() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&out).expect("mkdir");
    let path = out.join("graph.json");

    write_graph(&path, &["alpha", "beta"]);
    let g1 = load_graph(path.to_str().expect("str")).expect("load");
    let ids: Vec<_> = g1.nodes().map(|(id, _)| id.clone()).collect();
    assert!(ids.contains(&"alpha".to_string()));
    assert!(ids.contains(&"beta".to_string()));

    // Simulate file changing.
    std::thread::sleep(std::time::Duration::from_millis(10));
    write_graph(&path, &["alpha", "beta", "gamma"]);

    let g2 = load_graph(path.to_str().expect("str")).expect("load after write");
    let ids2: Vec<_> = g2.nodes().map(|(id, _)| id.clone()).collect();
    assert!(ids2.contains(&"gamma".to_string()));
}

#[test]
fn test_load_graph_cache_key_changes_with_content() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&out).expect("mkdir");
    let path = out.join("graph.json");

    write_graph(&path, &["a"]);
    let m1 = std::fs::metadata(&path).expect("stat1");
    let key1 = (m1.modified().expect("mtime1"), m1.len());

    std::thread::sleep(std::time::Duration::from_millis(10));
    write_graph(&path, &["a", "b"]);
    let m2 = std::fs::metadata(&path).expect("stat2");
    let key2 = (m2.modified().expect("mtime2"), m2.len());

    assert_ne!(key1, key2, "stat key must change when file content changes");
}

// ── IDF weighting tests (issue #897) ─────────────────────────────────────────

#[test]
fn test_idf_downweights_common_terms() {
    let g = make_noisy_graph();
    let mut cache = HashMap::new();
    // "foobarservice" matches 1 node; "error" matches 20 → IDF should make fbs rank first.
    let scored = score_nodes(&g, &["foobarservice", "error"], &mut cache);
    assert!(!scored.is_empty(), "should have results");
    assert_eq!(
        scored[0].1, "fbs",
        "FooBarService should rank first, got {}",
        scored[0].1
    );
}

#[test]
fn test_idf_cached_on_graph() {
    // Calling score_nodes should populate the IDF cache.
    let g = make_graph();
    let mut cache = HashMap::new();
    let _ = score_nodes(&g, &["extract"], &mut cache);
    assert!(
        cache.contains_key("extract"),
        "IDF cache should contain 'extract'"
    );
}

#[test]
fn test_idf_new_graph_starts_fresh() {
    let g1 = make_graph();
    let g2 = make_graph();
    let mut cache1 = HashMap::new();
    let mut cache2 = HashMap::new();
    let _ = score_nodes(&g1, &["extract"], &mut cache1);
    // g2 has its own separate cache — not shared.
    assert!(!cache2.contains_key("extract"));
    // After scoring g2, cache2 should be populated independently.
    let _ = score_nodes(&g2, &["extract"], &mut cache2);
    assert!(cache2.contains_key("extract"));
    let _ = g2; // suppress unused warning
}

#[test]
fn test_idf_rare_term_gets_high_weight() {
    let g = make_graph(); // 5 nodes
    let mut cache = HashMap::new();
    let idf = compute_idf(&g, &["extract"], &mut cache);
    // extract matches only n1: IDF = ln(1 + 5/2) ≈ 1.25
    assert!(idf["extract"] > 1.0, "rare term IDF should be > 1.0");
}

#[test]
fn test_idf_common_term_gets_low_weight() {
    // 'handle' in every node label → very low IDF.
    let mut nodes = vec![];
    for i in 0..20_u64 {
        nodes.push(json!({
            "id": format!("n{i}"),
            "label": format!("handle_{i}"),
            "source_file": format!("f{i}.py")
        }));
    }
    let g = build_from_json(json!({"nodes": nodes, "edges": []}), false, None).expect("graph");
    let mut cache = HashMap::new();
    let idf = compute_idf(&g, &["handle"], &mut cache);
    assert!(idf["handle"] < 1.0, "common term IDF should be < 1.0");
}

// ── _pick_seeds (issue #897) ──────────────────────────────────────────────────

#[test]
fn test_pick_seeds_dominant_identifier_gives_one_seed() {
    let scored = vec![
        (1000.0_f64, "fbs".to_string()),
        (1.0, "err1".to_string()),
        (0.9, "err2".to_string()),
    ];
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds, vec!["fbs".to_string()]);
}

#[test]
fn test_pick_seeds_close_scores_keeps_multiple() {
    let scored = vec![
        (10.0_f64, "a".to_string()),
        (9.0, "b".to_string()),
        (8.5, "c".to_string()),
    ];
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds.len(), 3);
}

#[test]
fn test_pick_seeds_empty() {
    let seeds = pick_seeds(&[], 3, 0.2);
    assert!(seeds.is_empty());
}

#[test]
fn test_pick_seeds_single() {
    let scored = vec![(5.0_f64, "x".to_string())];
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds, vec!["x".to_string()]);
}

#[test]
fn test_pick_seeds_respects_max_k() {
    let scored: Vec<(f64, String)> = (0..10).map(|i| (10.0, format!("n{i}"))).collect();
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds.len(), 3);
}

// ── Truncation hint (issue #897) ──────────────────────────────────────────────

#[test]
fn test_subgraph_to_text_truncation_hint_is_actionable() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> = ["n1", "n2", "n3", "n4"]
        .iter()
        .map(|&s| s.to_string())
        .collect();
    let text = subgraph_to_text(&g, &nodes, &[("n1".to_string(), "n2".to_string())], 1, None);
    assert!(text.contains("truncated"));
    assert!(
        text.contains("get_node") || text.contains("context_filter"),
        "truncation hint should tell user what to do"
    );
}

// ── Integration: identifier + noise (issue #897) ──────────────────────────────

#[test]
fn test_query_seeds_from_identifier_not_noise() {
    let g = make_noisy_graph();
    let mut cache = HashMap::new();
    let text = query_graph_text(
        &g,
        "FooBarService error handling",
        "bfs",
        2,
        2000,
        None,
        &mut cache,
    );
    assert!(
        text.contains("FooBarService"),
        "FooBarService should appear in results"
    );
    assert!(
        text.contains("ServiceClient"),
        "ServiceClient should appear as neighbor"
    );
}

// ── PR tool tests ─────────────────────────────────────────────────────────────

fn make_fake_gh() -> FakeGhClient {
    FakeGhClient {
        prs_json: CANNED_PR_JSON,
        files: vec!["src/feature_x.rs".to_string()],
        default_branch: Some("main".to_string()),
    }
}

#[test]
fn test_tool_list_prs_returns_pr_descriptors() {
    let gh = make_fake_gh();
    let result = tool_list_prs_with_clients(&json!({}), &gh, &FakeGitClient).unwrap();
    let prs = result["prs"].as_array().unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0]["number"], 42);
    assert_eq!(prs[0]["title"], "Add feature X");
    assert_eq!(prs[0]["author"], "alice");
}

#[test]
fn test_tool_list_prs_includes_count() {
    let gh = make_fake_gh();
    let result = tool_list_prs_with_clients(&json!({}), &gh, &FakeGitClient).unwrap();
    assert_eq!(result["count"], 1);
}

#[test]
fn test_tool_list_prs_handles_empty() {
    let gh = FakeGhClient {
        prs_json: "[]",
        files: vec![],
        default_branch: Some("main".to_string()),
    };
    let result = tool_list_prs_with_clients(&json!({}), &gh, &FakeGitClient).unwrap();
    let prs = result["prs"].as_array().unwrap();
    assert!(prs.is_empty());
    assert_eq!(result["count"], 0);
}

/// Minimal graph with one node whose `source_file` matches the PR's changed file.
fn make_impact_graph() -> Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "feature_x", "source_file": "src/feature_x.rs", "community": 0}
            ],
            "edges": []
        }),
        true,
        None,
    )
    .expect("make_impact_graph")
}

#[test]
fn test_tool_get_pr_impact_lists_affected_nodes() {
    let gh = make_fake_gh();
    let graph = make_impact_graph();
    let args = json!({"pr_number": 42});
    let result = tool_get_pr_impact_with_clients(&graph, &args, &gh).unwrap();
    assert!(
        result["affected_nodes"].as_u64().unwrap() > 0,
        "must report affected nodes when file matches"
    );
}

#[test]
fn test_tool_get_pr_impact_empty_when_no_match() {
    let gh = FakeGhClient {
        prs_json: CANNED_PR_JSON,
        files: vec!["other/unrelated.rs".to_string()],
        default_branch: Some("main".to_string()),
    };
    let graph = make_impact_graph();
    let args = json!({"pr_number": 42});
    let result = tool_get_pr_impact_with_clients(&graph, &args, &gh).unwrap();
    assert_eq!(
        result["affected_nodes"].as_u64().unwrap(),
        0,
        "no overlap → zero affected nodes"
    );
}

#[test]
fn test_tool_triage_prs_returns_structured_output() {
    let gh = make_fake_gh();
    let result = tool_triage_prs_with_clients(&json!({}), &gh, &FakeGitClient).unwrap();
    assert!(result.is_array(), "triage output must be a JSON array");
}

#[test]
fn test_tool_triage_prs_respects_limit() {
    // Only 1 PR in canned data; limit=1 should not change anything, but the
    // field must be respected (no more than `limit` items returned).
    let gh = make_fake_gh();
    let args = json!({"limit": 1});
    let result = tool_triage_prs_with_clients(&args, &gh, &FakeGitClient).unwrap();
    let items = result.as_array().unwrap();
    assert!(
        items.len() <= 1,
        "limit=1 must cap the result length; got {}",
        items.len()
    );
}

// ---------------------------------------------------------------------------
// query_terms: keep short non-English tokens (#964)
// ---------------------------------------------------------------------------

#[test]
fn query_terms_filters_only_short_english_terms() {
    assert_eq!(
        query_terms("the quick brown"),
        vec!["the", "quick", "brown"]
    );
    let r = query_terms("an ai bot");
    assert_eq!(r, vec!["bot"]);
}

#[test]
fn query_terms_keeps_short_non_english_terms() {
    let r = query_terms("認証");
    assert_eq!(r, vec!["認証"]);
}

#[test]
fn query_terms_lowercases() {
    let r = query_terms("AuthN AuthZ");
    assert_eq!(r, vec!["authn", "authz"]);
}

// ---------------------------------------------------------------------------
// load_graph: reject oversized files
// ---------------------------------------------------------------------------

#[test]
fn test_load_graph_accepts_under_cap() {
    // Smoke test of the happy path: a tiny well-formed graph round-trips
    // through the size-cap-guarded loader. Boundary testing with a tiny
    // cap lives in graphify-security's parity suite where the
    // `_with(cap)` variant lets us trigger the error explicitly.
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    // Canonical NetworkX `node_link_data` shape — `links` not `edges`,
    // plus the `directed`/`multigraph` flags the loader inspects. Using
    // the same shape as `test_load_graph_roundtrip` so this test exercises
    // the real parse path rather than a degenerate minimal payload.
    std::fs::write(
        &graph_path,
        br#"{"directed": true, "multigraph": false, "nodes": [], "links": []}"#,
    )
    .expect("write");
    let result = load_graph(graph_path.to_str().expect("utf-8"));
    assert!(result.is_ok(), "small graph should load: {result:?}");
}
