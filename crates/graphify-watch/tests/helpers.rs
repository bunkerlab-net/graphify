//! Targeted tests for small helpers exposed by the watch crate.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::process::Command;

use graphify_watch::{
    apply_resource_limits, check_shrink, git_head, node_community_map, relativize_source_files,
};
use serde_json::json;

// ── apply_resource_limits ────────────────────────────────────────────────────

#[test]
fn apply_resource_limits_runs_without_panicking() {
    // Best-effort; just verify it doesn't crash.
    apply_resource_limits();
}

#[test]
fn apply_resource_limits_with_memory_limit_env() {
    // SAFETY: test-only env var manipulation.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("GRAPHIFY_REBUILD_MEMORY_LIMIT_MB", "1024");
    }
    apply_resource_limits();
    #[allow(unsafe_code)]
    unsafe {
        std::env::remove_var("GRAPHIFY_REBUILD_MEMORY_LIMIT_MB");
    }
}

#[test]
fn apply_resource_limits_with_invalid_memory_env() {
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("GRAPHIFY_REBUILD_MEMORY_LIMIT_MB", "not-a-number");
    }
    apply_resource_limits();
    #[allow(unsafe_code)]
    unsafe {
        std::env::remove_var("GRAPHIFY_REBUILD_MEMORY_LIMIT_MB");
    }
}

// ── git_head ─────────────────────────────────────────────────────────────────

#[test]
fn git_head_returns_none_outside_repo() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(git_head(tmp.path()).is_none());
}

#[test]
fn git_head_returns_hash_in_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path();
    // Initialise minimal git repo.
    let out = Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .output()
        .unwrap();
    if !out.status.success() {
        // git not installed — skip.
        return;
    }

    // Configure identity to avoid CI failures.
    Command::new("git")
        .args(["config", "user.email", "t@example.com"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(path)
        .output()
        .unwrap();

    fs::write(path.join("a.txt"), "hi\n").unwrap();
    Command::new("git")
        .args(["add", "a.txt"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-qm", "init"])
        .current_dir(path)
        .output()
        .unwrap();

    let head = git_head(path).expect("git_head should return a hash");
    assert_eq!(head.len(), 40, "expected 40-char SHA, got {head:?}");
}

// ── relativize_source_files ──────────────────────────────────────────────────

#[test]
fn relativize_rewrites_absolute_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let file = root.join("sub").join("foo.py");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "x = 1\n").unwrap();

    let mut payload = json!({
        "nodes": [{"id": "a", "source_file": file.to_string_lossy()}],
        "edges": [{"source": "a", "target": "b", "source_file": file.to_string_lossy()}],
        "hyperedges": [],
    });
    relativize_source_files(&mut payload, &root);
    let new_path = payload["nodes"][0]["source_file"].as_str().unwrap();
    assert!(
        new_path == "sub/foo.py" || new_path.ends_with("foo.py"),
        "expected relative path, got {new_path}"
    );
}

#[test]
fn relativize_leaves_relative_paths_alone() {
    let mut payload = json!({
        "nodes": [{"id": "a", "source_file": "rel/path.py"}],
        "edges": [],
    });
    let root = std::env::current_dir().unwrap();
    relativize_source_files(&mut payload, &root);
    assert_eq!(payload["nodes"][0]["source_file"], "rel/path.py");
}

#[test]
fn relativize_handles_missing_source_file() {
    let mut payload = json!({
        "nodes": [{"id": "a"}],
        "edges": [],
    });
    let root = std::env::current_dir().unwrap();
    relativize_source_files(&mut payload, &root);
    // Should remain unchanged.
    assert!(payload["nodes"][0].get("source_file").is_none());
}

#[test]
fn relativize_noop_on_non_object_payload() {
    let mut payload = json!([1, 2, 3]);
    let root = std::env::current_dir().unwrap();
    relativize_source_files(&mut payload, &root);
    assert_eq!(payload, json!([1, 2, 3]));
}

// ── check_shrink ─────────────────────────────────────────────────────────────

#[test]
fn check_shrink_allows_growth() {
    let existing = json!({"nodes": [{"id": "a"}]});
    let new = json!({"nodes": [{"id": "a"}, {"id": "b"}]});
    assert!(check_shrink(false, &existing, &new, None).is_ok());
}

#[test]
fn check_shrink_allows_same() {
    let existing = json!({"nodes": [{"id": "a"}]});
    let new = json!({"nodes": [{"id": "b"}]});
    assert!(check_shrink(false, &existing, &new, None).is_ok());
}

#[test]
fn check_shrink_refuses_shrink() {
    let existing = json!({"nodes": [{"id": "a"}, {"id": "b"}]});
    let new = json!({"nodes": [{"id": "a"}]});
    assert!(check_shrink(false, &existing, &new, None).is_err());
}

#[test]
fn check_shrink_force_overrides() {
    let existing = json!({"nodes": [{"id": "a"}, {"id": "b"}]});
    let new = json!({"nodes": [{"id": "a"}]});
    assert!(check_shrink(true, &existing, &new, None).is_ok());
}

#[test]
fn check_shrink_no_existing_passes() {
    let existing = json!({"nodes": []});
    let new = json!({"nodes": [{"id": "a"}]});
    assert!(check_shrink(false, &existing, &new, None).is_ok());
}

#[test]
fn check_shrink_cleans_up_tmp_file_on_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().join("graph.tmp.json");
    fs::write(&tmp_path, "{}").unwrap();
    let existing = json!({"nodes": [{"id": "a"}, {"id": "b"}]});
    let new = json!({"nodes": [{"id": "a"}]});
    assert!(check_shrink(false, &existing, &new, Some(&tmp_path)).is_err());
    assert!(!tmp_path.exists(), "tmp file should be cleaned up");
}

// ── node_community_map ───────────────────────────────────────────────────────

#[test]
fn node_community_map_reads_graph_data() {
    let graph = json!({
        "nodes": [
            {"id": "a", "community": 1},
            {"id": "b", "community": 1},
            {"id": "c", "community": 2},
        ]
    });
    let map = node_community_map(&graph);
    assert_eq!(map.get("a"), Some(&1));
    assert_eq!(map.get("b"), Some(&1));
    assert_eq!(map.get("c"), Some(&2));
}

#[test]
fn node_community_map_handles_missing_community() {
    let graph = json!({
        "nodes": [
            {"id": "a"},
            {"id": "b", "community": 5},
        ]
    });
    let map = node_community_map(&graph);
    assert!(!map.contains_key("a"));
    assert_eq!(map.get("b"), Some(&5));
}

#[test]
fn node_community_map_handles_invalid_community_type() {
    let graph = json!({
        "nodes": [
            {"id": "a", "community": "not-a-number"},
        ]
    });
    let map = node_community_map(&graph);
    assert!(map.is_empty());
}

#[test]
fn node_community_map_returns_empty_for_missing_nodes() {
    let graph = json!({});
    let map = node_community_map(&graph);
    assert!(map.is_empty());
}
