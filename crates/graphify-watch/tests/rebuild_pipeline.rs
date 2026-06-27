//! Integration tests for the rebuild pipeline.
//!
//! Drives `rebuild_code` end-to-end against a temp directory containing a
//! small synthetic codebase, exercising detect → extract → build → cluster →
//! report → export.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use graphify_watch::{LockPolicy, RebuildOptions, rebuild_code};

// GRAPHIFY_OUT isolation: these tests drive `rebuild_code` against per-test
// tempdirs and read the default `graphify-out/` output dir. They deliberately
// do not isolate `GRAPHIFY_OUT` — `cargo nextest` runs each test in its own
// process, no test in this crate mutates `GRAPHIFY_OUT`, and `#[serial]` would
// not guard against an ambient override (shared equally by every test here
// that asserts on `graphify-out`). The `#[serial]` marks further down isolate
// `set_current_dir`, not the environment.

/// Parse `graph.json` at `path` and collect the given string field from every node.
fn node_field_set(path: &Path, field: &str) -> std::collections::HashSet<String> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(path).expect("read graph.json"))
            .expect("parse graph.json");
    value["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .filter_map(|n| n.get(field).and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

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
    let before = node_field_set(&graph_path, "label");
    assert!(
        before.contains("format_date()"),
        "format_date should be present before deletion"
    );

    fs::remove_file(corpus.join("utils.py")).expect("remove utils.py");
    assert!(rebuild_code(corpus, None, opts).expect("second rebuild"));

    let after = node_field_set(&graph_path, "label");
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
fn rebuild_code_evicts_removed_symbol_from_surviving_file() {
    // #1116: a symbol removed from a re-extracted (not deleted) file is a
    // legitimate shrink — `graphify update` must refresh the graph WITHOUT
    // --force, because every lost node belongs to a rebuilt source.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(
        corpus.join("auth.py"),
        "def login(): pass\ndef logout(): pass\ndef reset(): pass\n",
    )
    .expect("write auth.py");

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));

    let graph_path = corpus.join("graphify-out").join("graph.json");
    let before = node_field_set(&graph_path, "label");
    assert!(
        before.contains("reset()"),
        "reset should be present before the edit"
    );

    // Remove one function from the surviving file and re-run a full update.
    fs::write(
        corpus.join("auth.py"),
        "def login(): pass\ndef logout(): pass\n",
    )
    .expect("rewrite auth.py");
    assert!(
        rebuild_code(corpus, None, opts).expect("second rebuild without --force"),
        "shrink-guard must allow a symbol removed from a rebuilt source"
    );

    let after = node_field_set(&graph_path, "label");
    assert!(
        !after.contains("reset()"),
        "removed symbol must be pruned without --force"
    );
    assert!(after.contains("login()"), "surviving symbol must be kept");
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
    let before_sources = node_field_set(&graph_path, "source_file");
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

    let after_sources = node_field_set(&graph_path, "source_file");
    assert!(
        !after_sources.iter().any(|s| s.ends_with("drop.py")),
        "deleted file's nodes should be pruned (sources: {after_sources:?})"
    );
    assert!(
        after_sources.iter().any(|s| s.ends_with("keep.py")),
        "surviving file's nodes should remain (sources: {after_sources:?})"
    );
}

// ── #777: .graphify_root stores the user-supplied path (portable) ───────────

#[test]
fn graphify_root_preserves_user_supplied_absolute_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    fs::create_dir_all(&corpus).expect("mkdir");
    fs::write(corpus.join("lib.py"), "def f(): pass\n").expect("write");

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    assert!(rebuild_code(&corpus, None, opts).expect("rebuild"));

    let saved = fs::read_to_string(corpus.join("graphify-out").join(".graphify_root"))
        .expect("read .graphify_root");
    // The user-supplied (un-canonicalised) path is preserved verbatim.
    assert_eq!(saved, corpus.to_string_lossy());
}

#[test]
#[serial_test::serial]
fn graphify_root_preserves_relative_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    fs::create_dir_all(&corpus).expect("mkdir");
    fs::write(corpus.join("lib.py"), "def f(): pass\n").expect("write");

    // nextest runs each test in its own process, so changing the CWD here is
    // isolated; restore it afterwards for the `cargo test` (shared-process) case.
    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&corpus).expect("chdir corpus");
    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    let result = rebuild_code(Path::new("."), None, opts);
    std::env::set_current_dir(prev).expect("restore cwd");
    assert!(result.expect("rebuild"));

    let saved = fs::read_to_string(corpus.join("graphify-out").join(".graphify_root"))
        .expect("read .graphify_root");
    assert_eq!(saved, ".", ".graphify_root must preserve the relative path");
}

// ── #1116: full rebuild prunes stale AST symbols from surviving files ────────

#[test]
fn full_rebuild_prunes_stale_ast_node_from_surviving_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    fs::create_dir_all(&corpus).expect("mkdir");
    fs::write(corpus.join("a.py"), "def keep(): pass\n").expect("write");

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    assert!(rebuild_code(&corpus, None, opts).expect("initial rebuild"));

    // Inject a stale AST-stamped ghost (source survives but symbol is gone) and
    // a marker-less semantic node on the same surviving file.
    let graph_path = corpus.join("graphify-out").join("graph.json");
    let mut data: serde_json::Value =
        serde_json::from_slice(&fs::read(&graph_path).expect("read")).expect("parse");
    let nodes = data["nodes"].as_array_mut().expect("nodes");
    nodes.push(serde_json::json!({
        "id": "a_ghostsym", "label": "GhostSym", "_origin": "ast",
        "file_type": "function", "source_file": "a.py",
    }));
    nodes.push(serde_json::json!({
        "id": "a_authconcept", "label": "AuthConcept",
        "file_type": "concept", "source_file": "a.py",
    }));
    fs::write(&graph_path, serde_json::to_string(&data).expect("ser")).expect("write");

    let opts = RebuildOptions {
        force: true,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    assert!(rebuild_code(&corpus, None, opts).expect("full rebuild"));

    let after = node_field_set(&graph_path, "label");
    assert!(!after.contains("GhostSym"), "stale AST node must be pruned");
    assert!(after.contains("AuthConcept"), "semantic node must be kept");
    assert!(after.contains("keep()"), "surviving symbol must be kept");
}

#[test]
fn full_rebuild_keeps_marker_less_stale_node_one_cycle() {
    // #1118 backward-compat: a node with no `_origin` marker (pre-upgrade graph)
    // is NOT pruned on the first full rebuild — a deliberate one-cycle lag.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    fs::create_dir_all(&corpus).expect("mkdir");
    fs::write(corpus.join("a.py"), "def keep(): pass\n").expect("write");

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    assert!(rebuild_code(&corpus, None, opts).expect("initial rebuild"));

    let graph_path = corpus.join("graphify-out").join("graph.json");
    let mut data: serde_json::Value =
        serde_json::from_slice(&fs::read(&graph_path).expect("read")).expect("parse");
    data["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(serde_json::json!({
            "id": "a_ghostml", "label": "GhostML",
            "file_type": "function", "source_file": "a.py",
        }));
    fs::write(&graph_path, serde_json::to_string(&data).expect("ser")).expect("write");

    let opts = RebuildOptions {
        force: true,
        no_cluster: false,
        lock: LockPolicy::None,
    };
    assert!(rebuild_code(&corpus, None, opts).expect("full rebuild"));

    let after = node_field_set(&graph_path, "label");
    assert!(
        after.contains("GhostML"),
        "marker-less stale node survives one cycle (no _origin marker)"
    );
}

// ── #1348: repo-relative changed paths resolve against a subdir-rooted graph ──

#[test]
#[serial_test::serial]
fn rebuild_code_accepts_repo_relative_changed_path_for_subdir_root() {
    // A git hook passes paths relative to the repository root even when the
    // graph is rooted at a subdirectory. `src/app.py` must resolve to the real
    // file under the watched `src` root, not `src/src/app.py` (which would look
    // like a deletion and wrongly evict the file's nodes).
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    let app = src.join("app.py");
    fs::write(&app, "def old_name():\n    return 1\n").expect("write app.py");

    let graph_path = src.join("graphify-out").join("graph.json");
    let prev = std::env::current_dir().expect("cwd");
    // nextest runs each test in its own process, so changing the CWD here is
    // isolated; the `#[serial]` attribute + restore below cover `cargo test`.
    std::env::set_current_dir(tmp.path()).expect("chdir tmp");

    let full = RebuildOptions {
        force: false,
        no_cluster: true,
        lock: LockPolicy::None,
    };
    let first = rebuild_code(Path::new("src"), None, full);
    let first_ok = matches!(first, Ok(true));
    let first_dbg = format!("{first:?}");
    let before = first_ok.then(|| node_field_set(&graph_path, "label"));

    let rewrite = fs::write(&app, "def new_name():\n    return 2\n");

    // The hook hands us a repo-root-relative path; the watched root is `src`.
    let incremental = RebuildOptions {
        force: true,
        no_cluster: true,
        lock: LockPolicy::None,
    };
    let second = rebuild_code(
        Path::new("src"),
        Some(&[PathBuf::from("src/app.py")]),
        incremental,
    );
    let second_ok = matches!(second, Ok(true));
    let second_dbg = format!("{second:?}");
    let after = second_ok.then(|| node_field_set(&graph_path, "label"));

    std::env::set_current_dir(prev).expect("restore cwd");

    assert!(
        first_ok,
        "first rebuild should report an update, got {first_dbg}"
    );
    rewrite.expect("rewrite app.py");
    assert!(
        second_ok,
        "incremental rebuild with a repo-relative changed path should update, got {second_dbg}"
    );

    let before = before.expect("first run must produce graph.json");
    assert!(
        before.contains("old_name()"),
        "first run should extract the original symbol (labels: {before:?})"
    );
    let after = after.expect("incremental run must produce graph.json");
    assert!(
        after.contains("new_name()"),
        "renamed symbol must be extracted via the repo-relative path (labels: {after:?})"
    );
    assert!(
        !after.contains("old_name()"),
        "stale symbol from the subdir-rooted file must be evicted (labels: {after:?})"
    );
}

// ── #1317: --no-cluster incremental re-extraction must not accumulate dup edges ──

/// Count the entries in `graph.json`'s `links` array.
fn link_count(path: &Path) -> usize {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(path).expect("read graph.json"))
            .expect("parse graph.json");
    value["links"].as_array().map_or(0, Vec::len)
}

#[test]
fn no_cluster_incremental_reextract_does_not_duplicate_edges() {
    // The clustered path collapses parallel edges via its DiGraph, but the
    // --no-cluster write path concatenates the fresh extraction with the
    // preserved edges from the prior graph. Re-extracting the same file would
    // double its intra-file edges (inherits / contains / method) without the
    // dedupe in run_no_cluster_path.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(
        corpus.join("mod.py"),
        "class Base:\n    def run(self):\n        return 1\n\n\nclass Child(Base):\n    def run(self):\n        return 2\n",
    )
    .expect("write mod.py");

    let opts = RebuildOptions {
        force: true,
        no_cluster: true,
        lock: LockPolicy::None,
    };

    // Full build.
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    let first_links = link_count(&graph_path);
    assert!(
        first_links >= 1,
        "fixture must yield at least one edge to exercise dedupe (got {first_links})"
    );

    // Incremental re-extraction of the same file: edges must not accumulate.
    let changed: Vec<PathBuf> = vec![corpus.join("mod.py")];
    assert!(rebuild_code(corpus, Some(&changed), opts).expect("incremental rebuild"));
    let second_links = link_count(&graph_path);
    assert_eq!(
        second_links, first_links,
        "re-extracting an unchanged file must not duplicate edges (#1317)"
    );
}
