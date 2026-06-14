//! Parity tests against `graphify-py/tests/test_global_graph.py`.
//!
//! Each test function mirrors a Python test case at the same name.
//!
//! Allow `expect_used` here — test code is allowed to panic with explicit
//! messages rather than propagate errors via `?`.
#![allow(clippy::expect_used)]

use graphify_build::{Graph, GraphKind};
use graphify_global::{
    global_add, global_list, global_remove, load_graph_from_file, prefix_graph_for_global,
    prune_repo_from_graph, save_graph_to_file,
};
use indexmap::IndexMap;
use serde_json::Value;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Helper: build a Graph from simple node/edge specs (mirrors Python `_make_graph`)
// ---------------------------------------------------------------------------

fn make_graph(nodes: &[(&str, &[(&str, &str)])], edges: &[(&str, &str)]) -> Graph {
    let mut g = Graph::new(GraphKind::Graph);
    for (id, attrs) in nodes {
        let mut map: IndexMap<String, Value> = IndexMap::new();
        for (k, v) in *attrs {
            map.insert((*k).to_string(), Value::String((*v).to_string()));
        }
        g.add_node(id, map);
    }
    for (src, tgt) in edges {
        g.add_edge(src, tgt, IndexMap::new());
    }
    g
}

/// Build an attribute map from `(key, value)` string pairs.
fn attrs(pairs: &[(&str, &str)]) -> IndexMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
        .collect()
}

// ---------------------------------------------------------------------------
// build helpers (prefix_graph_for_global / prune_repo_from_graph)
// ---------------------------------------------------------------------------

// Mirrors: test_prefix_graph_preserves_label
#[test]
fn test_prefix_graph_preserves_label() {
    let g = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );
    let h = prefix_graph_for_global(&g, "repoA");

    assert!(h.contains_node("repoA::userservice"));
    assert!(!h.contains_node("userservice"));
    let data = h.node_data("repoA::userservice").expect("test invariant");
    assert_eq!(
        data.get("label").and_then(Value::as_str),
        Some("UserService")
    );
}

// Mirrors: test_prefix_graph_sets_repo_and_local_id
#[test]
fn test_prefix_graph_sets_repo_and_local_id() {
    let g = make_graph(&[("userservice", &[("label", "UserService")])], &[]);
    let h = prefix_graph_for_global(&g, "repoA");
    let data = h.node_data("repoA::userservice").expect("test invariant");
    assert_eq!(data.get("repo").and_then(Value::as_str), Some("repoA"));
    assert_eq!(
        data.get("local_id").and_then(Value::as_str),
        Some("userservice")
    );
}

// Mirrors: test_prefix_graph_rewrites_edges
#[test]
fn test_prefix_graph_rewrites_edges() {
    let g = make_graph(
        &[("a", &[("label", "A")]), ("b", &[("label", "B")])],
        &[("a", "b")],
    );
    let h = prefix_graph_for_global(&g, "repo1");

    assert!(h.edge_data("repo1::a", "repo1::b").is_some());
    assert!(h.edge_data("a", "b").is_none());
}

// Mirrors: test_prune_repo_removes_correct_nodes
#[test]
fn test_prune_repo_removes_correct_nodes() {
    let mut g = Graph::new(GraphKind::Graph);
    for (id, repo) in [
        ("repoA::userservice", "repoA"),
        ("repoB::userservice", "repoB"),
        ("repoA::auth", "repoA"),
    ] {
        let mut attrs = IndexMap::new();
        attrs.insert("repo".to_string(), Value::String(repo.to_string()));
        g.add_node(id, attrs);
    }
    let removed = prune_repo_from_graph(&mut g, "repoA");
    assert_eq!(removed, 2);
    assert!(g.contains_node("repoB::userservice"));
    assert!(!g.contains_node("repoA::userservice"));
    assert!(!g.contains_node("repoA::auth"));
}

// Mirrors: test_prune_repo_returns_zero_if_not_present
#[test]
fn test_prune_repo_returns_zero_if_not_present() {
    let mut g = Graph::new(GraphKind::Graph);
    let mut attrs = IndexMap::new();
    attrs.insert("repo".to_string(), Value::String("repoA".to_string()));
    g.add_node("repoA::x", attrs);

    let removed = prune_repo_from_graph(&mut g, "repoB");
    assert_eq!(removed, 0);
    assert_eq!(g.node_count(), 1);
}

// ---------------------------------------------------------------------------
// global_graph operations
// ---------------------------------------------------------------------------

// Helper: write a Graph to a tempdir file and return the path.
fn write_graph_file(dir: &std::path::Path, name: &str, graph: &Graph) -> std::path::PathBuf {
    let path = dir.join(name);
    save_graph_to_file(&path, graph).expect("save_graph_to_file failed");
    path
}

// Mirrors: test_global_add_creates_global_graph
#[test]
fn test_global_add_creates_global_graph() {
    let tmp = tempdir().expect("tempdir");
    let g = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );
    let src = write_graph_file(tmp.path(), "graph.json", &g);

    let global_dir = tmp.path().join(".graphify");
    let graph_path = global_dir.join("global-graph.json");
    let manifest_path = global_dir.join("global-manifest.json");

    let result = global_add(&src, "repoA", &graph_path, &manifest_path).expect("test invariant");

    assert!(!result.skipped);
    assert!(result.nodes_added > 0);
    assert!(manifest_path.exists());
    let repos = global_list(&manifest_path);
    assert!(repos.contains_key("repoA"));
}

// Mirrors: test_global_add_skip_on_unchanged_hash
#[test]
fn test_global_add_skip_on_unchanged_hash() {
    let tmp = tempdir().expect("tempdir");
    let g = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );
    let src = write_graph_file(tmp.path(), "graph.json", &g);

    let global_dir = tmp.path().join(".graphify");
    let graph_path = global_dir.join("global-graph.json");
    let manifest_path = global_dir.join("global-manifest.json");

    global_add(&src, "repoA", &graph_path, &manifest_path).expect("test invariant");
    let result2 = global_add(&src, "repoA", &graph_path, &manifest_path).expect("test invariant");

    assert!(result2.skipped);
}

// Mirrors: test_global_add_two_repos_no_collision
#[test]
fn test_global_add_two_repos_no_collision() {
    let tmp = tempdir().expect("tempdir");
    let g1 = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );
    let g2 = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );
    let src1 = write_graph_file(tmp.path(), "graph1.json", &g1);
    let src2 = write_graph_file(tmp.path(), "graph2.json", &g2);

    let global_dir = tmp.path().join(".graphify");
    let graph_path = global_dir.join("global-graph.json");
    let manifest_path = global_dir.join("global-manifest.json");

    global_add(&src1, "repoA", &graph_path, &manifest_path).expect("test invariant");
    global_add(&src2, "repoB", &graph_path, &manifest_path).expect("test invariant");

    let merged = load_graph_from_file(&graph_path).expect("test invariant");
    assert!(merged.contains_node("repoA::userservice"));
    assert!(merged.contains_node("repoB::userservice"));
    assert_eq!(merged.node_count(), 2); // no silent collapse
}

// Mirrors: test_global_add_rewires_edges_to_deduplicated_externals
#[test]
fn test_global_add_rewires_edges_to_deduplicated_externals() {
    let tmp = tempdir().expect("tempdir");

    let mut ga = Graph::new(GraphKind::Graph);
    ga.add_node(
        "moda",
        attrs(&[("label", "ModA"), ("source_file", "src/a.py")]),
    );
    ga.add_node("requests", attrs(&[("label", "requests")]));
    ga.add_edge("moda", "requests", attrs(&[("relation", "imports")]));

    let mut gb = Graph::new(GraphKind::Graph);
    gb.add_node(
        "modb",
        attrs(&[("label", "ModB"), ("source_file", "src/b.py")]),
    );
    gb.add_node("requests", attrs(&[("label", "requests")]));
    gb.add_edge("modb", "requests", attrs(&[("relation", "imports")]));

    let src1 = write_graph_file(tmp.path(), "graph1.json", &ga);
    let src2 = write_graph_file(tmp.path(), "graph2.json", &gb);

    let global_dir = tmp.path().join(".graphify");
    let graph_path = global_dir.join("global-graph.json");
    let manifest_path = global_dir.join("global-manifest.json");

    global_add(&src1, "repoA", &graph_path, &manifest_path).expect("test invariant");
    global_add(&src2, "repoB", &graph_path, &manifest_path).expect("test invariant");

    let merged = load_graph_from_file(&graph_path).expect("test invariant");

    // repoB's external "requests" was deduplicated against repoA's.
    assert!(merged.contains_node("repoA::requests"));
    assert!(!merged.contains_node("repoB::requests"));
    // repoA's edge is untouched.
    assert!(merged.edge_data("repoA::moda", "repoA::requests").is_some());
    // repoB's edge must be rewired to the existing external node, not dropped.
    let rewired = merged
        .edge_data("repoB::modb", "repoA::requests")
        .expect("rewired edge present");
    assert_eq!(
        rewired.get("relation").and_then(Value::as_str),
        Some("imports")
    );
}

// Rust-side regression: a corrupt global manifest is backed up (not silently
// wiped) so the user does not lose every tracked repo. Mirrors the data-loss
// fix in `global_graph._load_manifest`.
#[test]
fn test_corrupt_manifest_backed_up_not_wiped() {
    let tmp = tempdir().expect("tempdir");
    let manifest_path = tmp.path().join("global-manifest.json");
    std::fs::write(&manifest_path, "{ not valid json").expect("write");

    // Reading through the public API returns the empty default ...
    let repos = global_list(&manifest_path);
    assert!(repos.is_empty());

    // ... and the corrupt original is moved aside to a timestamped backup.
    assert!(!manifest_path.exists());
    let backups = std::fs::read_dir(tmp.path())
        .expect("readdir")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("global-manifest.json.corrupt.")
        })
        .count();
    assert_eq!(backups, 1);
}

// Mirrors: test_global_remove
#[test]
fn test_global_remove() {
    let tmp = tempdir().expect("tempdir");
    let g = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );
    let src = write_graph_file(tmp.path(), "graph.json", &g);

    let global_dir = tmp.path().join(".graphify");
    let graph_path = global_dir.join("global-graph.json");
    let manifest_path = global_dir.join("global-manifest.json");

    global_add(&src, "repoA", &graph_path, &manifest_path).expect("test invariant");
    let removed = global_remove("repoA", &graph_path, &manifest_path).expect("test invariant");

    assert!(removed > 0);
    let repos = global_list(&manifest_path);
    assert!(!repos.contains_key("repoA"));
}

// Mirrors: test_global_remove_unknown_tag_raises
#[test]
fn test_global_remove_unknown_tag_raises() {
    let tmp = tempdir().expect("tempdir");
    let global_dir = tmp.path().join(".graphify");
    let graph_path = global_dir.join("global-graph.json");
    let manifest_path = global_dir.join("global-manifest.json");

    let err = global_remove("nonexistent", &graph_path, &manifest_path)
        .expect_err("should fail for unknown repo");
    assert!(
        err.to_string().contains("nonexistent"),
        "error message should mention the tag: {err}"
    );
}

// Mirrors: test_global_add_collision_warning
// The Python test asserts a warning appears on stderr; we verify the call
// succeeds without panicking when the source path changes (even if content is
// the same and the result is skipped — skipping is correct when hash matches).
#[test]
fn test_global_add_collision_different_source_path() {
    let tmp = tempdir().expect("tempdir");
    let g = make_graph(&[("x", &[("label", "X"), ("source_file", "x.py")])], &[]);
    let src1 = write_graph_file(tmp.path(), "graph1.json", &g);
    // Write a graph with different content so the hash won't match.
    let g2 = make_graph(
        &[
            ("x", &[("label", "X"), ("source_file", "x.py")]),
            ("y", &[("label", "Y"), ("source_file", "y.py")]),
        ],
        &[],
    );
    let src2 = write_graph_file(tmp.path(), "graph2.json", &g2);

    let global_dir = tmp.path().join(".graphify");
    let graph_path = global_dir.join("global-graph.json");
    let manifest_path = global_dir.join("global-manifest.json");

    global_add(&src1, "myrepo", &graph_path, &manifest_path).expect("test invariant");
    // Different source path and content — should warn and proceed (not skipped).
    let result = global_add(&src2, "myrepo", &graph_path, &manifest_path).expect("test invariant");
    assert!(!result.skipped);
}

// Mirrors: test_merge_graphs_prefixes_ids
#[test]
fn test_merge_graphs_prefixes_ids() {
    let g1 = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );
    let g2 = make_graph(
        &[(
            "userservice",
            &[("label", "UserService"), ("source_file", "src/user.py")],
        )],
        &[],
    );

    let p1 = prefix_graph_for_global(&g1, "repo1");
    let p2 = prefix_graph_for_global(&g2, "repo2");

    // Compose by merging both into a fresh graph (mirrors nx.compose).
    let mut merged = Graph::new(GraphKind::Graph);
    for (id, attrs) in p1.nodes().chain(p2.nodes()) {
        merged.add_node(id, attrs.clone());
    }

    assert!(merged.contains_node("repo1::userservice"));
    assert!(merged.contains_node("repo2::userservice"));
    assert_eq!(merged.node_count(), 2); // no silent collapse
}
