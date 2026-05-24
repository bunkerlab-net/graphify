//! Integration tests for the rebuild pipeline.
//!
//! Drives `rebuild_code` end-to-end against a temp directory containing a
//! small synthetic codebase, exercising detect → extract → build → cluster →
//! report → export.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use graphify_watch::{LockPolicy, RebuildOptions, rebuild_code};

/// Create a small Python project in `dir`.
fn write_python_project(dir: &Path) {
    let src = dir.join("src");
    fs::create_dir_all(&src).unwrap();

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
    .unwrap();

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
    .unwrap();
}

#[test]
fn rebuild_code_produces_graph_and_report() {
    let tmp = tempfile::tempdir().unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    rebuild_code(tmp.path(), None, opts).unwrap();
    // Second call should still succeed (idempotent) without errors.
    let _ = rebuild_code(tmp.path(), None, opts).unwrap();

    let graph = tmp.path().join("graphify-out").join("graph.json");
    assert!(graph.exists());
}

#[test]
fn rebuild_code_with_no_cluster_path() {
    let tmp = tempfile::tempdir().unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
    // Only put a README.md (document, not code).
    fs::write(tmp.path().join("README.md"), "# nothing\n").unwrap();

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    // README.md actually has a markdown extractor — see helpers::detect_code_files.
    // To get an empty code set we need to use a totally extension-less file.
    fs::remove_file(tmp.path().join("README.md")).unwrap();
    fs::write(tmp.path().join("notes"), "plain text\n").unwrap();

    let updated = rebuild_code(tmp.path(), None, opts).unwrap();
    assert!(!updated, "rebuild without code files should return false");
}

#[test]
fn rebuild_code_with_changed_paths() {
    let tmp = tempfile::tempdir().unwrap();
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    // First full rebuild.
    rebuild_code(tmp.path(), None, opts).unwrap();

    // Now do an incremental rebuild with a specific changed file.
    let changed: Vec<PathBuf> = vec![tmp.path().join("src/models.py")];
    let _ = rebuild_code(tmp.path(), Some(&changed), opts).unwrap();

    let out = tmp.path().join("graphify-out");
    assert!(out.join("graph.json").exists());
}

#[test]
fn rebuild_code_with_force_flag() {
    let tmp = tempfile::tempdir().unwrap();
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: true,
        no_cluster: false,
        lock: LockPolicy::None,
    };

    let updated = rebuild_code(tmp.path(), None, opts).unwrap();
    assert!(updated);
}

#[test]
fn rebuild_code_with_try_acquire_lock() {
    let tmp = tempfile::tempdir().unwrap();
    write_python_project(tmp.path());

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::TryAcquire,
    };

    let updated = rebuild_code(tmp.path(), None, opts).unwrap();
    assert!(updated);
}
