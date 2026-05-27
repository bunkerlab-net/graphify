//! Integration tests for the rebuild pipeline.
//!
//! Drives `rebuild_code` end-to-end against a temp directory containing a
//! small synthetic codebase, exercising detect → extract → build → cluster →
//! report → export.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use graphify_watch::{LockPolicy, RebuildOptions, rebuild_code};

/// Create a small Python project in `dir`.
fn write_python_project(dir: &Path) {
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("create_dir_all");

    fs::write(
        src.join("models.py"),
        r"
class User:
    def __init__(self, name):
        self.name = name

    def greet(self):
        return f'Hello, {self.name}'

class Admin(User):
    def ban(self, other):
        return f'banned {other}'
",
    )
    .expect("test invariant");

    fs::write(
        src.join("main.py"),
        r"
from src.models import User, Admin

def make_admin(name):
    return Admin(name)

def main():
    u = make_admin('alice')
    print(u.greet())

if __name__ == '__main__':
    main()
",
    )
    .expect("test invariant");
}

#[test]
fn rebuild_code_produces_graph_and_report() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    let updated = rebuild_code(tmp.path(), None, opts).expect("rebuild_code should succeed");
    assert!(updated, "first rebuild should report an update");

    let out = tmp.path().join("graphify-out");
    assert!(out.join("graph.json").exists(), "graph.json missing");
    assert!(
        out.join("GRAPH_REPORT.md").exists(),
        "GRAPH_REPORT.md missing"
    );
    assert!(
        out.join(".graphify_root").exists(),
        ".graphify_root marker missing"
    );
}

#[test]
fn rebuild_code_idempotent_when_topology_unchanged() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    rebuild_code(tmp.path(), None, opts).expect("test invariant");
    // Second call should still succeed (idempotent) without errors.
    let _ = rebuild_code(tmp.path(), None, opts).expect("test invariant");

    let graph = tmp.path().join("graphify-out").join("graph.json");
    assert!(graph.exists());
}

#[test]
fn rebuild_code_with_no_cluster_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: true,
        lock: LockPolicy::None,
    };

    let updated = rebuild_code(tmp.path(), None, opts).expect("no_cluster rebuild should succeed");
    assert!(updated);

    let out = tmp.path().join("graphify-out");
    assert!(out.join("graph.json").exists());
}

#[test]
fn rebuild_code_returns_false_when_no_code_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Only put a README.md (document, not code).
    fs::write(tmp.path().join("README.md"), "# nothing\n").expect("test invariant");

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    // README.md actually has a markdown extractor — see helpers::detect_code_files.
    // To get an empty code set we need to use a totally extension-less file.
    fs::remove_file(tmp.path().join("README.md")).expect("test invariant");
    fs::write(tmp.path().join("notes"), "plain text\n").expect("test invariant");

    let updated = rebuild_code(tmp.path(), None, opts).expect("test invariant");
    assert!(!updated, "rebuild without code files should return false");
}

#[test]
fn rebuild_code_with_changed_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    // First full rebuild.
    rebuild_code(tmp.path(), None, opts).expect("test invariant");

    // Now do an incremental rebuild with a specific changed file.
    let changed: Vec<PathBuf> = vec![tmp.path().join("src/models.py")];
    let _ = rebuild_code(tmp.path(), Some(&changed), opts).expect("test invariant");

    let out = tmp.path().join("graphify-out");
    assert!(out.join("graph.json").exists());
}

#[test]
fn rebuild_code_evicts_nodes_from_deleted_files() {
    // #1007: `graphify update` (full re-extraction, changed_paths=None) must
    // remove nodes and edges from files deleted since the last run.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(
        corpus.join("auth.py"),
        "def login(): pass\ndef logout(): pass\n",
    )
    .expect("write auth.py");
    fs::write(corpus.join("utils.py"), "def format_date(): pass\n").expect("write utils.py");

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));

    let graph_path = corpus.join("graphify-out").join("graph.json");
    let node_labels = |path: &Path| -> std::collections::HashSet<String> {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("read graph")).expect("parse graph.json");
        value["nodes"]
            .as_array()
            .expect("nodes array")
            .iter()
            .filter_map(|n| n.get("label").and_then(|v| v.as_str()).map(str::to_string))
            .collect()
    };

    let before = node_labels(&graph_path);
    assert!(
        before.contains("format_date()"),
        "format_date should be present before deletion"
    );

    fs::remove_file(corpus.join("utils.py")).expect("remove utils.py");
    assert!(rebuild_code(corpus, None, opts).expect("second rebuild"));

    let after = node_labels(&graph_path);
    assert!(
        !after.contains("format_date()"),
        "stale function node from deleted file must be evicted"
    );
    assert!(
        after.contains("login()"),
        "node from surviving file must be kept"
    );
}

#[test]
fn rebuild_code_with_force_flag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: true,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    let updated = rebuild_code(tmp.path(), None, opts).expect("test invariant");
    assert!(updated);
}

#[test]
fn rebuild_code_with_try_acquire_lock() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::TryAcquire,
    };

    let updated = rebuild_code(tmp.path(), None, opts).expect("test invariant");
    assert!(updated);
}

/// End-to-end probe of the explicit-deletion bypass.
///
/// Mirrors `tests/test_watch.py::test_rebuild_code_prunes_deleted_file_nodes`:
/// build a graph from two files, delete one, then call `rebuild_code` with the
/// deleted path in `changed_paths`. The post-commit hook does this whenever a
/// commit removes a tracked file. Without the bypass the shrink guard would
/// refuse to overwrite; with the bypass the deleted file's nodes are pruned
/// and the surviving file's nodes remain.
#[test]
fn rebuild_code_prunes_deleted_file_nodes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let keep = tmp.path().join("keep.py");
    let drop_file = tmp.path().join("drop.py");
    fs::write(&keep, "def keep_fn():\n    return 1\n").expect("write keep.py");
    fs::write(&drop_file, "def drop_fn():\n    return 2\n").expect("write drop.py");

    let opts = RebuildOptions {
        force: false,
        no_cluster: true,
        lock: LockPolicy::None,
    };

    // Initial build covers both files.
    let updated = rebuild_code(tmp.path(), None, opts).expect("initial rebuild");
    assert!(updated);
    let graph_path = tmp.path().join("graphify-out").join("graph.json");
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&graph_path).expect("read graph.json"))
            .expect("parse graph.json");
    let before_sources: std::collections::HashSet<String> = before["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .filter_map(|n| {
            n.get("source_file")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(
        before_sources.iter().any(|s| s.ends_with("drop.py")),
        "drop.py should appear before deletion (sources: {before_sources:?})"
    );

    // Delete drop.py and re-run with the path in the change list.
    std::fs::remove_file(&drop_file).expect("remove drop.py");
    let updated = rebuild_code(tmp.path(), Some(&[PathBuf::from("drop.py")]), opts)
        .expect("rebuild after deletion should succeed");
    assert!(
        updated,
        "rebuild should succeed even though the graph shrinks"
    );

    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&graph_path).expect("read graph.json"))
            .expect("parse graph.json");
    let after_sources: std::collections::HashSet<String> = after["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .filter_map(|n| {
            n.get("source_file")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(
        !after_sources.iter().any(|s| s.ends_with("drop.py")),
        "deleted file's nodes should be pruned (sources: {after_sources:?})"
    );
    assert!(
        after_sources.iter().any(|s| s.ends_with("keep.py")),
        "surviving file's nodes should remain (sources: {after_sources:?})"
    );
}
