//! Integration tests for the rebuild pipeline.
//!
//! Drives `rebuild_code` end-to-end against a temp directory containing a
//! small synthetic codebase, exercising detect → extract → build → cluster →
//! report → export.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use graphify_watch::{LockPolicy, RebuildOptions, rebuild_code, write_build_config};

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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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
        follow_symlinks: false,
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

#[test]
fn rebuild_code_prunes_a_removed_imports_edge() {
    // #1521: when an import is deleted from a file, a full `update` must prune the
    // edge it produced — preserving it (keyed only on endpoint membership) left a
    // stale edge that drove phantom circular-dependency findings.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    let pkg = corpus.join("pkg");
    fs::create_dir_all(&pkg).expect("mkdir pkg");
    fs::write(pkg.join("b.py"), "def helper():\n    return 1\n").expect("write b.py");
    fs::write(
        pkg.join("a.py"),
        "from pkg.b import helper\ndef use():\n    return helper()\n",
    )
    .expect("write a.py");

    let opts = RebuildOptions {
        force: false,
        no_cluster: true,
        lock: LockPolicy::None,
        follow_symlinks: false,
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));

    let graph_path = corpus.join("graphify-out").join("graph.json");
    let a_import_edges = |path: &Path| -> usize {
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("read graph.json")).expect("parse");
        v.get("links")
            .or_else(|| v.get("edges"))
            .and_then(|e| e.as_array())
            .map_or(0, |edges| {
                edges
                    .iter()
                    .filter(|e| {
                        matches!(
                            e.get("relation").and_then(|r| r.as_str()),
                            Some("imports" | "imports_from")
                        ) && e
                            .get("source_file")
                            .and_then(|s| s.as_str())
                            .is_some_and(|s| s.ends_with("a.py"))
                    })
                    .count()
            })
    };
    assert!(
        a_import_edges(&graph_path) > 0,
        "expected an import edge from a.py initially"
    );

    // Remove the import, then run a full update (changed_paths = None).
    fs::write(pkg.join("a.py"), "def use():\n    return 1\n").expect("rewrite a.py");
    assert!(rebuild_code(corpus, None, opts).expect("second rebuild"));
    assert_eq!(
        a_import_edges(&graph_path),
        0,
        "removed import's edge owned by a.py must be pruned"
    );
}

#[test]
#[serial_test::serial]
fn rebuild_code_recovers_from_deleted_cwd_via_repo_root() {
    // e5044c3: a detached git hook can inherit a working directory that is
    // deleted before the background rebuild starts. When GRAPHIFY_REPO_ROOT
    // names the repo, rebuild_code chdir's there and still succeeds.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    fs::create_dir_all(&corpus).expect("mkdir corpus");
    fs::write(corpus.join("lib.py"), "def f():\n    return 1\n").expect("write lib.py");
    let doomed = tmp.path().join("doomed");
    fs::create_dir_all(&doomed).expect("mkdir doomed");

    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&doomed).expect("chdir doomed");
    // SAFETY: test-only env manipulation, serialized by #[serial].
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("GRAPHIFY_REPO_ROOT", &corpus);
    }
    // Delete the CWD out from under the process, as a transient hook dir would be.
    let _ = fs::remove_dir_all(&doomed);

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
        follow_symlinks: false,
    };
    let result = rebuild_code(Path::new("."), Some(&[PathBuf::from("lib.py")]), opts);

    // Restore a valid CWD + env before asserting so teardown is safe.
    std::env::set_current_dir(&prev).expect("restore cwd");
    #[allow(unsafe_code)]
    unsafe {
        std::env::remove_var("GRAPHIFY_REPO_ROOT");
    }

    assert!(
        result.expect("rebuild recovers via repo root"),
        "rebuild should recover from a deleted CWD via GRAPHIFY_REPO_ROOT"
    );
    assert!(
        corpus.join("graphify-out").join("graph.json").exists(),
        "graph.json written under the recovered repo root"
    );
}

#[test]
#[serial_test::serial]
fn rebuild_code_deleted_cwd_without_repo_root_returns_false() {
    // e5044c3: with the CWD gone and no GRAPHIFY_REPO_ROOT, the rebuild fails
    // cleanly (Ok(false)) rather than panicking on a relative graphify-out mkdir.
    let tmp = tempfile::tempdir().expect("tempdir");
    let doomed = tmp.path().join("doomed");
    fs::create_dir_all(&doomed).expect("mkdir doomed");
    let prev = std::env::current_dir().expect("cwd");
    // SAFETY: test-only env manipulation, serialized by #[serial].
    #[allow(unsafe_code)]
    unsafe {
        std::env::remove_var("GRAPHIFY_REPO_ROOT");
    }
    std::env::set_current_dir(&doomed).expect("chdir doomed");
    let _ = fs::remove_dir_all(&doomed);

    let opts = RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
        follow_symlinks: false,
    };
    let result = rebuild_code(Path::new("."), Some(&[PathBuf::from("lib.py")]), opts);

    // Restore a valid CWD before asserting so teardown is safe.
    std::env::set_current_dir(&prev).expect("restore cwd");
    assert_eq!(
        result.ok(),
        Some(false),
        "a deleted CWD with no repo root must skip cleanly"
    );
}

#[test]
fn rebuild_preserves_surviving_hyperedges_on_full_rebuild() {
    // #1755: a full rebuild re-extracts every file, but hyperedges from
    // surviving (non-deleted) sources must be preserved — only deleted-source or
    // dangling ones are dropped. The old code reused the edge eviction set (which
    // on a full rebuild covers every file), wiping every hyperedge.
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("a.py"), "def f():\n    return 1\n").expect("write a.py");
    let opts = RebuildOptions {
        force: false,
        no_cluster: true,
        lock: LockPolicy::None,
        follow_symlinks: false,
    };
    assert!(rebuild_code(tmp.path(), None, opts).expect("first rebuild"));

    let graph_path = tmp.path().join("graphify-out").join("graph.json");
    let mut data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&graph_path).expect("read graph")).expect("parse");
    let node_id = data["nodes"][0]["id"]
        .as_str()
        .expect("at least one node")
        .to_string();
    // A semantic hyperedge from a surviving file, anchored on a real node.
    data["hyperedges"] = serde_json::json!([{
        "id": "hyper-1",
        "nodes": [node_id],
        "source_file": "a.py",
        "relation": "co_change",
    }]);
    fs::write(
        &graph_path,
        serde_json::to_string(&data).expect("serialize"),
    )
    .expect("write graph");

    // Add a second file so the full rebuild genuinely changes (and rewrites) the
    // graph, forcing the reconcile path to run.
    fs::write(tmp.path().join("b.py"), "def g():\n    return 2\n").expect("write b.py");
    assert!(rebuild_code(tmp.path(), None, opts).expect("second rebuild"));

    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&graph_path).expect("read graph")).expect("parse");
    let hypers = after["hyperedges"].as_array().expect("hyperedges array");
    assert!(
        hypers.iter().any(|h| h["id"].as_str() == Some("hyper-1")),
        "surviving hyperedge must persist across a full rebuild: {hypers:?}"
    );
}

// ── #8d8d2b8: reconcile removed and renamed sources ──────────────────────────

/// Parse `graph.json` into a mutable JSON value.
fn read_graph(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).expect("read graph")).expect("parse graph")
}

/// Write a JSON value back to `graph.json`.
fn write_graph(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec(value).expect("serialize graph")).expect("write graph");
}

/// Seed an unrelated semantic node pair + edge + hyperedge that must survive a
/// reconcile (they carry no AST origin and no live source file). Mirrors
/// `_add_unrelated_semantic_pair`.
fn add_unrelated_semantic_pair(path: &Path) {
    use serde_json::json;
    let mut data = read_graph(path);
    let nodes = data["nodes"].as_array_mut().expect("nodes");
    nodes.push(json!({"id": "docs_topic", "label": "DocsTopic", "file_type": "concept"}));
    nodes.push(json!({"id": "shared_concept", "label": "SharedConcept", "file_type": "concept"}));
    data["links"].as_array_mut().expect("links").push(
        json!({"source": "docs_topic", "target": "shared_concept", "relation": "related_to"}),
    );
    data["hyperedges"] = json!([{
        "id": "semantic_context",
        "label": "Semantic context",
        "nodes": ["docs_topic", "shared_concept"],
    }]);
    write_graph(path, &data);
}

fn node_ids(data: &serde_json::Value) -> std::collections::HashSet<String> {
    data["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

/// #1755 (specific): AST-only updates must not drop a semantic hyperedge whose
/// members survive. Ports `test_rebuild_code_preserves_hyperedges_for_rebuilt_surviving_source`.
fn preserves_hyperedges_for_rebuilt_surviving_source(changed_paths: Option<&[PathBuf]>) {
    use serde_json::json;
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(corpus.join("doc.md"), "# Design\n\n## Flow\n\nDetails.\n").expect("write doc.md");
    let opts = RebuildOptions {
        no_cluster: true,
        ..RebuildOptions::default()
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    let ids = node_ids(&read_graph(&graph_path));
    assert!(
        ids.contains("doc") && ids.contains("doc_design"),
        "markdown extraction must yield doc/doc_design: {ids:?}"
    );
    let hyper = json!({
        "id": "doc_flow_group",
        "label": "Doc flow group",
        "nodes": ["doc", "doc_design"],
        "relation": "implements",
        "confidence": "EXTRACTED",
        "confidence_score": 1.0,
        "source_file": "doc.md",
    });
    let mut data = read_graph(&graph_path);
    data["hyperedges"] = json!([hyper.clone()]);
    write_graph(&graph_path, &data);

    assert!(rebuild_code(corpus, changed_paths, opts).expect("second rebuild"));
    let after = read_graph(&graph_path);
    assert_eq!(after["hyperedges"], json!([hyper]));
}

#[test]
fn rebuild_preserves_hyperedges_for_rebuilt_surviving_source_full() {
    preserves_hyperedges_for_rebuilt_surviving_source(None);
}

#[test]
fn rebuild_preserves_hyperedges_for_rebuilt_surviving_source_incremental() {
    preserves_hyperedges_for_rebuilt_surviving_source(Some(&[PathBuf::from("doc.md")]));
}

/// Deleting the final code file must reconcile the prior graph: prune the code
/// node, drop its owning hyperedge + a sourceless AST stub, keep the unrelated
/// semantic pair. Ports `test_rebuild_code_prunes_final_deleted_file`.
fn prunes_final_deleted_file(changed_paths: Option<&[PathBuf]>) {
    use serde_json::json;
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    let only = corpus.join("only.py");
    fs::write(&only, "def only_fn():\n    return 1\n").expect("write only.py");
    let opts = RebuildOptions {
        no_cluster: true,
        ..RebuildOptions::default()
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    add_unrelated_semantic_pair(&graph_path);
    let mut before = read_graph(&graph_path);
    let code_node_id = before["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n.get("source_file").and_then(|v| v.as_str()) == Some("only.py"))
        .and_then(|n| n.get("id").and_then(|v| v.as_str()))
        .expect("code node id")
        .to_string();
    before["hyperedges"]
        .as_array_mut()
        .expect("hyper")
        .push(json!({
            "id": "code_context",
            "label": "Code context",
            "nodes": [code_node_id],
            "source_file": "only.py",
        }));
    before["nodes"].as_array_mut().expect("nodes").push(json!({
        "id": "sourceless_ast_stub",
        "label": "ExternalType",
        "file_type": "class",
        "_origin": "ast",
    }));
    write_graph(&graph_path, &before);

    fs::remove_file(&only).expect("unlink only.py");
    assert!(rebuild_code(corpus, changed_paths, opts).expect("second rebuild"));

    let after = read_graph(&graph_path);
    let sources: std::collections::HashSet<String> = after["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| {
            n.get("source_file")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(!sources.contains("only.py"), "deleted source pruned");
    let ids = node_ids(&after);
    assert!(ids.contains("docs_topic") && ids.contains("shared_concept"));
    let links = after["links"].as_array().expect("links");
    assert!(links.iter().any(
        |e| e.get("source").and_then(|v| v.as_str()) == Some("docs_topic")
            && e.get("target").and_then(|v| v.as_str()) == Some("shared_concept")
    ));
    let hyper_ids: std::collections::HashSet<String> = after["hyperedges"]
        .as_array()
        .expect("hyper")
        .iter()
        .filter_map(|h| h.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    assert_eq!(
        hyper_ids,
        std::collections::HashSet::from(["semantic_context".to_string()])
    );
    assert!(
        !ids.contains("sourceless_ast_stub"),
        "sourceless AST stub dropped"
    );
}

#[test]
fn rebuild_prunes_final_deleted_file_full() {
    prunes_final_deleted_file(None);
}

#[test]
fn rebuild_prunes_final_deleted_file_incremental() {
    prunes_final_deleted_file(Some(&[PathBuf::from("only.py")]));
}

/// A hook-style rename list may contain only the destination path; the stale
/// source is still evicted. Ports `test_rebuild_code_prunes_renamed_source_not_listed_by_hook`.
#[test]
fn rebuild_prunes_renamed_source_not_listed_by_hook() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    let old = corpus.join("old.py");
    fs::write(&old, "def old_fn():\n    return 1\n").expect("write old.py");
    let opts = RebuildOptions {
        no_cluster: true,
        ..RebuildOptions::default()
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    add_unrelated_semantic_pair(&graph_path);

    fs::rename(&old, corpus.join("renamed.py")).expect("rename");
    assert!(
        rebuild_code(corpus, Some(&[PathBuf::from("renamed.py")]), opts).expect("second rebuild")
    );

    let after = read_graph(&graph_path);
    let sources: std::collections::HashSet<String> = after["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| {
            n.get("source_file")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(!sources.contains("old.py"), "renamed-away source evicted");
    assert!(sources.contains("renamed.py"), "new source present");
    let ids = node_ids(&after);
    assert!(ids.contains("docs_topic") && ids.contains("shared_concept"));
}

/// `./foo.py` in a preserved node must not be treated as a deleted live source.
/// Ports `test_rebuild_code_normalizes_preserved_source_paths`.
#[test]
fn rebuild_normalizes_preserved_source_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(corpus.join("foo.py"), "def foo_fn():\n    return 1\n").expect("write foo.py");
    fs::write(corpus.join("bar.py"), "def bar_fn():\n    return 1\n").expect("write bar.py");
    let opts = RebuildOptions {
        no_cluster: true,
        ..RebuildOptions::default()
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    let mut data = read_graph(&graph_path);
    for bucket in ["nodes", "links"] {
        for item in data[bucket].as_array_mut().expect("bucket") {
            if item.get("source_file").and_then(|v| v.as_str()) == Some("foo.py") {
                item["source_file"] = serde_json::Value::String("./foo.py".to_string());
            }
        }
    }
    write_graph(&graph_path, &data);

    fs::write(
        corpus.join("bar.py"),
        "def updated_bar_fn():\n    return 2\n",
    )
    .expect("update bar");
    assert!(rebuild_code(corpus, Some(&[PathBuf::from("bar.py")]), opts).expect("second rebuild"));

    let after = read_graph(&graph_path);
    let labels: std::collections::HashSet<String> = after["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n.get("label").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    assert!(
        labels.contains("foo_fn()"),
        "./foo.py must not be pruned as deleted: {labels:?}"
    );
}

/// Destination-only rename reconciliation covers AST-backed documents too.
/// Ports `test_rebuild_code_prunes_renamed_ast_backed_document`.
#[test]
fn rebuild_prunes_renamed_ast_backed_document() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    let old = corpus.join("old.md");
    fs::write(&old, "# Old heading\n").expect("write old.md");
    let opts = RebuildOptions {
        no_cluster: true,
        ..RebuildOptions::default()
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    fs::rename(&old, corpus.join("renamed.md")).expect("rename");
    assert!(
        rebuild_code(corpus, Some(&[PathBuf::from("renamed.md")]), opts).expect("second rebuild")
    );
    let after = read_graph(&graph_path);
    let sources: std::collections::HashSet<String> = after["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| {
            n.get("source_file")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(!sources.contains("old.md"), "old doc source evicted");
    assert!(sources.contains("renamed.md"), "renamed doc source present");
}

/// A full/incremental rebuild of a subdirectory must not prune graph data that
/// lives outside it (an AST node whose source is a sibling file), but must drop
/// a stale AST node whose subdir source is gone. Ports
/// `test_rebuild_code_subdir_preserves_outside_ast_nodes`.
fn subdir_preserves_outside_ast_nodes(changed_paths: Option<&[PathBuf]>) {
    use serde_json::json;
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    fs::write(
        tmp.path().join("app.py"),
        "def outside_fn():\n    return 2\n",
    )
    .expect("outside");
    fs::write(src.join("app.py"), "def inside_fn():\n    return 1\n").expect("inside");

    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(tmp.path()).expect("chdir tmp");
    let result = std::panic::catch_unwind(|| {
        let opts = RebuildOptions {
            no_cluster: true,
            ..RebuildOptions::default()
        };
        assert!(rebuild_code(Path::new("src"), None, opts).expect("first rebuild"));
        let graph_path = src.join("graphify-out").join("graph.json");
        let mut data = read_graph(&graph_path);
        let inside_id = data["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .find(|n| n.get("label").and_then(|v| v.as_str()) == Some("inside_fn()"))
            .and_then(|n| n.get("id").and_then(|v| v.as_str()))
            .expect("inside id")
            .to_string();
        let nodes = data["nodes"].as_array_mut().expect("nodes");
        nodes.push(json!({"id": "outside_ast", "label": "outside_fn()", "file_type": "function", "source_file": "app.py", "_origin": "ast"}));
        nodes.push(json!({"id": "stale_inside_ast", "label": "stale_inside_fn()", "file_type": "function", "source_file": "src/deleted.py", "_origin": "ast"}));
        data["links"].as_array_mut().expect("links").push(json!({"source": "outside_ast", "target": inside_id, "relation": "calls", "source_file": "app.py"}));
        write_graph(&graph_path, &data);

        assert!(rebuild_code(Path::new("src"), changed_paths, opts).expect("second rebuild"));
        let after = read_graph(&graph_path);
        let ids = node_ids(&after);
        assert!(
            ids.contains("outside_ast"),
            "sibling-file AST node preserved"
        );
        assert!(
            !ids.contains("stale_inside_ast"),
            "stale subdir AST node dropped"
        );
        let outside = after["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("outside_ast"))
            .expect("outside node")
            .clone();
        assert_eq!(outside["source_file"], "app.py");
    });
    std::env::set_current_dir(&prev).expect("restore cwd");
    result.expect("subdir test panicked");
}

#[test]
#[serial_test::serial]
fn rebuild_subdir_preserves_outside_ast_nodes_full() {
    subdir_preserves_outside_ast_nodes(None);
}

#[test]
#[serial_test::serial]
fn rebuild_subdir_preserves_outside_ast_nodes_incremental() {
    subdir_preserves_outside_ast_nodes(Some(&[PathBuf::from("src/app.py")]));
}

/// Persisted relative source paths keep their meaning when the invocation style
/// changes (absolute dir then `Path("src")`), and a later rename reconciles.
/// Ports `test_rebuild_code_subdir_survives_absolute_to_relative_invocation`.
#[test]
#[serial_test::serial]
fn rebuild_subdir_survives_absolute_to_relative_invocation() {
    use serde_json::json;
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    let old = src.join("old.py");
    fs::write(&old, "def old_fn():\n    return 1\n").expect("write old.py");
    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(tmp.path()).expect("chdir tmp");
    let result = std::panic::catch_unwind(|| {
        let opts = RebuildOptions {
            no_cluster: true,
            ..RebuildOptions::default()
        };
        // First: invoke with the absolute src dir.
        assert!(rebuild_code(&src, None, opts).expect("abs rebuild"));
        let graph_path = src.join("graphify-out").join("graph.json");
        let mut data = read_graph(&graph_path);
        data["nodes"].as_array_mut().expect("nodes").push(json!({"id": "local_semantic", "label": "LocalSemantic", "file_type": "concept", "source_file": "old.py"}));
        write_graph(&graph_path, &data);

        // Then: invoke with the relative `src` path. The preserved semantic node's
        // source rebases to `src/old.py`.
        assert!(rebuild_code(Path::new("src"), None, opts).expect("rel rebuild"));
        let rebased = read_graph(&graph_path);
        let semantic = rebased["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("local_semantic"))
            .expect("semantic")
            .clone();
        assert_eq!(semantic["source_file"], "src/old.py");

        fs::rename(&old, src.join("renamed.py")).expect("rename");
        assert!(rebuild_code(Path::new("src"), None, opts).expect("rename rebuild"));
        let after = read_graph(&graph_path);
        let sources: std::collections::HashSet<String> = after["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .filter_map(|n| {
                n.get("source_file")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();
        assert!(!sources.contains("old.py"));
        assert!(sources.contains("src/renamed.py"));
    });
    std::env::set_current_dir(&prev).expect("restore cwd");
    result.expect("invocation test panicked");
}

/// A pre-rebase subdir graph stored `source_file` relative to `watch_root`; the
/// legacy detection still evicts a renamed-away source. Ports
/// `test_rebuild_code_prunes_legacy_watch_relative_subdir_source`.
#[test]
#[serial_test::serial]
fn rebuild_prunes_legacy_watch_relative_subdir_source() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    let old = src.join("old.py");
    fs::write(&old, "def old_fn():\n    return 1\n").expect("write old.py");
    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(tmp.path()).expect("chdir tmp");
    let result = std::panic::catch_unwind(|| {
        let opts = RebuildOptions {
            no_cluster: true,
            ..RebuildOptions::default()
        };
        assert!(rebuild_code(Path::new("src"), None, opts).expect("first rebuild"));
        let graph_path = src.join("graphify-out").join("graph.json");
        // Strip the `src/` prefix to emulate a pre-rebase watch-relative graph.
        let mut data = read_graph(&graph_path);
        for bucket in ["nodes", "links"] {
            for item in data[bucket].as_array_mut().expect("bucket") {
                if let Some(s) = item.get("source_file").and_then(|v| v.as_str())
                    && let Some(stripped) = s.strip_prefix("src/")
                {
                    item["source_file"] = serde_json::Value::String(stripped.to_string());
                }
            }
        }
        write_graph(&graph_path, &data);

        fs::rename(&old, src.join("renamed.py")).expect("rename");
        assert!(
            rebuild_code(
                Path::new("src"),
                Some(&[PathBuf::from("src/renamed.py")]),
                opts
            )
            .expect("second rebuild")
        );
        let after = read_graph(&graph_path);
        let sources: std::collections::HashSet<String> = after["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .filter_map(|n| {
                n.get("source_file")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();
        assert!(
            !sources.contains("old.py"),
            "legacy watch-relative source evicted"
        );
        assert!(sources.contains("src/renamed.py"), "renamed source present");
    });
    std::env::set_current_dir(&prev).expect("restore cwd");
    result.expect("legacy test panicked");
}

/// A rejected candidate keeps the `.graphify_root` marker paired with the
/// existing graph. Ports `test_rebuild_code_does_not_update_root_marker_when_write_is_refused`.
#[test]
#[serial_test::serial]
fn rebuild_does_not_update_root_marker_when_write_is_refused() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    let app = src.join("app.py");
    fs::write(&app, "def before():\n    return 1\n").expect("write app.py");
    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(tmp.path()).expect("chdir tmp");
    let result = std::panic::catch_unwind(|| {
        let opts = RebuildOptions {
            no_cluster: true,
            ..RebuildOptions::default()
        };
        assert!(rebuild_code(&src, None, opts).expect("first rebuild"));
        let marker = src.join("graphify-out").join(".graphify_root");
        assert_eq!(
            fs::read_to_string(&marker).expect("marker"),
            src.to_string_lossy()
        );

        fs::write(&app, "def after():\n    return 2\n").expect("edit app.py");
        // Force a shrink refusal via the typed test seam (no env / global state).
        let refused = graphify_watch::test_support::rebuild_code_forcing_shrink_refusal(
            Path::new("src"),
            None,
            opts,
        );
        // Rust surfaces a shrink refusal as `Err(ShrinkRefused)` (established
        // crate-wide via `check_shrink`'s `?`), where graphify-py returns False;
        // either way the write is refused, which is what this test pins.
        assert!(refused.is_err(), "shrink refusal must abort the write");
        assert_eq!(
            fs::read_to_string(&marker).expect("marker"),
            src.to_string_lossy(),
            "marker must still name the original root after a refused write"
        );
    });
    std::env::set_current_dir(&prev).expect("restore cwd");
    result.expect("marker test panicked");
}

/// Changed files under followed symlinks retain their watched lexical path
/// across renames. Ports `test_rebuild_code_incremental_rename_preserves_symlink_source_path`.
#[cfg(unix)]
#[test]
fn rebuild_incremental_rename_preserves_symlink_source_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    let real = corpus.join("real");
    fs::create_dir_all(&real).expect("mkdir real");
    fs::write(corpus.join(".graphifyignore"), "real/\n").expect("write ignore");
    let old = real.join("old.py");
    fs::write(&old, "def linked_fn():\n    return 1\n").expect("write old.py");
    std::os::unix::fs::symlink(&real, corpus.join("linked")).expect("symlink");

    let opts = RebuildOptions {
        no_cluster: true,
        follow_symlinks: true,
        ..RebuildOptions::default()
    };
    assert!(rebuild_code(corpus, None, opts).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");

    let first = real.join("first.py");
    fs::rename(&old, &first).expect("rename to first");
    assert!(rebuild_code(corpus, Some(&[PathBuf::from("linked/first.py")]), opts).expect("second"));

    let second = real.join("second.py");
    fs::rename(&first, &second).expect("rename to second");
    assert!(rebuild_code(corpus, Some(&[PathBuf::from("linked/second.py")]), opts).expect("third"));

    let after = read_graph(&graph_path);
    let sources: std::collections::HashSet<String> = after["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| {
            n.get("source_file")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(
        !sources.contains("linked/old.py"),
        "old symlink path evicted"
    );
    assert!(
        !sources.contains("linked/first.py"),
        "intermediate symlink path evicted"
    );
    assert!(
        sources.contains("linked/second.py"),
        "current symlink path present: {sources:?}"
    );
}

/// `rebuild_code` must never mutate the caller's working directory: path
/// resolution now roots relative paths explicitly rather than `chdir`-ing, so a
/// concurrent caller's CWD is never disturbed (`CodeRabbit` review follow-up).
#[test]
#[serial_test::serial]
fn rebuild_code_does_not_mutate_cwd() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_python_project(tmp.path());
    let before = std::env::current_dir().expect("cwd");
    let opts = RebuildOptions {
        no_cluster: true,
        ..RebuildOptions::default()
    };
    // Pass the tempdir as an absolute path so the rebuild does real work.
    assert!(rebuild_code(tmp.path(), None, opts).expect("rebuild"));
    let after = std::env::current_dir().expect("cwd");
    assert_eq!(
        before, after,
        "rebuild_code must not change the process CWD"
    );
}

// ── U7: persisted excludes, community names, edge tiers, fail-closed ──────────

fn default_opts() -> RebuildOptions {
    RebuildOptions {
        force: false,
        no_cluster: false,
        lock: LockPolicy::None,
        follow_symlinks: false,
    }
}

fn no_cluster_opts() -> RebuildOptions {
    RebuildOptions {
        no_cluster: true,
        ..default_opts()
    }
}

fn source_file_set(path: &Path) -> std::collections::HashSet<String> {
    node_field_set(path, "source_file")
}

#[test]
fn rebuild_honors_persisted_excludes() {
    // #1886: `--exclude` recorded at extract time must survive into rebuilds.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::create_dir_all(corpus.join("src")).expect("mkdir src");
    fs::create_dir_all(corpus.join("vendor")).expect("mkdir vendor");
    fs::write(corpus.join("src/app.py"), "def keep(): return 1\n").expect("write");
    fs::write(corpus.join("main.py"), "def top(): return 2\n").expect("write");
    fs::write(corpus.join("vendor/lib.py"), "def vendored(): pass\n").expect("write");
    write_build_config(&corpus.join("graphify-out"), Some(&["vendor".to_string()]));

    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("rebuild"));

    let sources = source_file_set(&corpus.join("graphify-out").join("graph.json"));
    assert!(sources.iter().any(|s| s.contains("src/app.py")));
    assert!(sources.iter().any(|s| s.contains("main.py")));
    assert!(
        !sources.iter().any(|s| s.contains("vendor/lib.py")),
        "rebuild silently re-included an excluded path (#1886): {sources:?}"
    );
}

#[test]
fn rebuild_code_writes_community_name() {
    // #1808: an update rebuild must forward community_labels so clustered nodes
    // carry a human-readable community_name, not just a numeric id.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(
        corpus.join("a.py"),
        "def alpha():\n    return beta()\n\ndef beta():\n    return 1\n",
    )
    .expect("write a.py");
    fs::write(
        corpus.join("b.py"),
        "import a\n\ndef gamma():\n    return a.alpha()\n",
    )
    .expect("write b.py");
    assert!(rebuild_code(corpus, None, default_opts()).expect("rebuild"));

    let graph = read_graph(&corpus.join("graphify-out").join("graph.json"));
    let clustered: Vec<&serde_json::Value> = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter(|n| !n.get("community").is_none_or(serde_json::Value::is_null))
        .collect();
    assert!(!clustered.is_empty(), "expected clustered nodes");
    assert!(
        clustered.iter().all(|n| n
            .get("community_name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|s| !s.is_empty())),
        "clustered nodes missing community_name (#1808)"
    );
}

#[test]
fn update_rebuilds_with_nested_star_gitignore() {
    // #1880: a nested `.gitignore` with a bare `*` must not zero the re-scan.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::create_dir_all(corpus.join("src")).expect("mkdir src");
    fs::write(
        corpus.join("src/a.py"),
        "from src.b import Base\nclass App(Base):\n    def run(self): return 1\n",
    )
    .expect("write");
    fs::write(corpus.join("src/b.py"), "class Base: pass\n").expect("write");
    fs::write(corpus.join("main.py"), "def top(): return 2\n").expect("write");
    fs::create_dir_all(corpus.join("scratch")).expect("mkdir scratch");
    fs::write(corpus.join("scratch/.gitignore"), "*\n").expect("write");
    fs::write(corpus.join("scratch/junk.py"), "x = 1\n").expect("write");

    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("rebuild"));

    let graph = read_graph(&corpus.join("graphify-out").join("graph.json"));
    let sources = source_file_set(&corpus.join("graphify-out").join("graph.json"));
    assert!(
        !graph["nodes"].as_array().expect("nodes").is_empty(),
        "update produced 0 nodes (#1880)"
    );
    assert!(sources.iter().any(|s| s.contains("src/a.py")));
    assert!(sources.iter().any(|s| s.contains("main.py")));
    assert!(!sources.iter().any(|s| s.contains("scratch/junk.py")));
}

#[test]
fn update_discovers_newly_added_files_and_dirs() {
    // #1837: a plain update (full re-scan) discovers brand-new files AND dirs.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::create_dir_all(corpus.join("src")).expect("mkdir src");
    fs::write(corpus.join("src/a.py"), "def alpha(): return 1\n").expect("write");
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("first rebuild"));

    fs::write(corpus.join("src/new.py"), "def added(): return 2\n").expect("write");
    fs::create_dir_all(corpus.join("monitor")).expect("mkdir monitor");
    fs::write(corpus.join("monitor/dash.py"), "def board(): return 3\n").expect("write");
    fs::create_dir_all(corpus.join("scratch")).expect("mkdir scratch");
    fs::write(corpus.join("scratch/.gitignore"), "*\n").expect("write");
    fs::write(corpus.join("scratch/junk.py"), "x = 1\n").expect("write");

    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("second rebuild"));

    let sources = source_file_set(&corpus.join("graphify-out").join("graph.json"));
    assert!(
        sources.iter().any(|s| s.contains("src/new.py")),
        "new file not discovered (#1837)"
    );
    assert!(
        sources.iter().any(|s| s.contains("monitor/dash.py")),
        "new dir not discovered (#1837)"
    );
    assert!(!sources.iter().any(|s| s.contains("scratch/junk.py")));
}

#[test]
fn rebuild_code_preserves_nodes_from_excluded_but_alive_file() {
    // #1795 fail-closed: a file that leaves the corpus (newly ignored) but still
    // exists on disk was EXCLUDED, not deleted — its nodes survive an incremental
    // rebuild. (Python also asserts the stdout "fail-closed: kept" message; we
    // emit it via println! but assert only the behavioural outcome, since a test
    // cannot capture its own process stdout.)
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::create_dir_all(corpus.join("notes")).expect("mkdir notes");
    fs::write(corpus.join("auth.py"), "def login(): pass\n").expect("write");
    fs::write(
        corpus.join("notes/brainstorm.md"),
        "# Brainstorm\n\nA local-only design note.\n",
    )
    .expect("write");
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    assert!(node_field_set(&graph_path, "label").contains("brainstorm.md"));

    // The file becomes ignored (leaves the corpus) but stays on disk.
    fs::write(corpus.join(".graphifyignore"), "notes/\n").expect("write ignore");
    assert!(
        rebuild_code(corpus, Some(&[PathBuf::from("auth.py")]), no_cluster_opts())
            .expect("second rebuild")
    );
    assert!(
        node_field_set(&graph_path, "label").contains("brainstorm.md"),
        "nodes from an excluded-but-alive file must be preserved, not evicted (#1795)"
    );
}

#[test]
fn rebuild_code_still_evicts_when_excluded_file_is_also_deleted() {
    // #1795: the fail-closed preserve must not weaken true-deletion eviction.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::create_dir_all(corpus.join("notes")).expect("mkdir notes");
    fs::write(corpus.join("auth.py"), "def login(): pass\n").expect("write");
    fs::write(corpus.join("notes/brainstorm.md"), "# Brainstorm\n").expect("write");
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");

    fs::remove_file(corpus.join("notes/brainstorm.md")).expect("unlink");
    assert!(
        rebuild_code(corpus, Some(&[PathBuf::from("auth.py")]), no_cluster_opts())
            .expect("second rebuild")
    );
    let labels = node_field_set(&graph_path, "label");
    assert!(
        !labels.contains("brainstorm.md"),
        "deleted file's nodes must still be evicted"
    );
    assert!(labels.contains("login()"));
}

fn preserves_semantic_edges_from_reextracted_doc(changed_paths: Option<&[PathBuf]>) {
    use serde_json::json;
    // #1865: an AST-only update must not evict semantic edges whose source is a
    // re-extracted document; only that source's AST-tier edges are replaced.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(
        corpus.join("auth.md"),
        "# Token Validation\n\nVerifies bearer tokens.\n",
    )
    .expect("write");
    fs::write(
        corpus.join("login.md"),
        "# Session Verification\n\nVerifies login sessions.\n",
    )
    .expect("write");
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("first rebuild"));
    let graph_path = corpus.join("graphify-out").join("graph.json");
    let mut data = read_graph(&graph_path);
    let ids = node_ids(&data);
    assert!(ids.contains("auth_token_validation") && ids.contains("login_session_verification"));

    let links = data["links"].as_array_mut().expect("links");
    links.push(json!({
        "source": "auth_token_validation", "target": "login_session_verification",
        "relation": "semantically_similar_to", "confidence": "INFERRED", "source_file": "auth.md",
    }));
    links.push(json!({
        "source": "auth_token_validation", "target": "login_session_verification",
        "relation": "references", "_origin": "ast", "source_file": "auth.md",
    }));
    write_graph(&graph_path, &data);

    assert!(rebuild_code(corpus, changed_paths, no_cluster_opts()).expect("second rebuild"));

    let after = read_graph(&graph_path);
    let relations: std::collections::HashSet<(String, String, String)> = after["links"]
        .as_array()
        .expect("links")
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                e.get("target")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                e.get("relation")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect();
    assert!(
        relations.contains(&(
            "auth_token_validation".to_string(),
            "login_session_verification".to_string(),
            "semantically_similar_to".to_string()
        )),
        "semantic edge from a re-extracted doc must survive an AST-only update"
    );
    assert!(
        !relations.contains(&(
            "auth_token_validation".to_string(),
            "login_session_verification".to_string(),
            "references".to_string()
        )),
        "stale AST-tier edge of a re-extracted source must be evicted"
    );
}

#[test]
fn rebuild_code_preserves_semantic_edges_from_reextracted_doc_full_update() {
    preserves_semantic_edges_from_reextracted_doc(None);
}

#[test]
fn rebuild_code_preserves_semantic_edges_from_reextracted_doc_incremental() {
    preserves_semantic_edges_from_reextracted_doc(Some(&[PathBuf::from("auth.md")]));
}

// ── #1915: semantic-backed docs must not be double-represented by AST scan ────

const SEMANTIC_GUIDE_IDS: [&str; 3] = ["guide_doc", "auth_flow", "session_model"];
const AST_GUIDE_IDS: [&str; 4] = ["guide", "guide_overview", "guide_setup", "guide_usage"];

/// Build a code-only graph, then add `guide.md` represented ONLY semantically
/// (a `_doc` node + concept nodes, none carrying `_origin`, no AST headings) —
/// mimicking a graph produced by the CLI update path.
fn seed_semantic_doc_graph(corpus: &Path) -> PathBuf {
    use serde_json::json;
    fs::write(corpus.join("app.py"), "def handle_login():\n    return 1\n").expect("write");
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("seed rebuild"));

    fs::write(
        corpus.join("guide.md"),
        "# Overview\n\nIntro.\n\n## Setup\n\nSteps.\n\n## Usage\n\nMore.\n",
    )
    .expect("write");
    let graph_path = corpus.join("graphify-out").join("graph.json");
    let mut data = read_graph(&graph_path);
    let code_node_id = data["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n.get("source_file").and_then(serde_json::Value::as_str) == Some("app.py"))
        .and_then(|n| n.get("id").and_then(serde_json::Value::as_str))
        .expect("code node")
        .to_string();
    let nodes = data["nodes"].as_array_mut().expect("nodes");
    nodes.push(json!({"id": "guide_doc", "label": "Guide", "file_type": "document", "source_file": "guide.md"}));
    nodes.push(json!({"id": "auth_flow", "label": "Auth Flow", "file_type": "concept", "source_file": "guide.md"}));
    nodes.push(json!({"id": "session_model", "label": "Session Model", "file_type": "concept", "source_file": "guide.md"}));
    let links = data["links"].as_array_mut().expect("links");
    links.push(json!({"source": "guide_doc", "target": "auth_flow", "relation": "explains", "confidence": "INFERRED", "source_file": "guide.md"}));
    links.push(json!({"source": "auth_flow", "target": code_node_id, "relation": "implemented_by", "confidence": "INFERRED", "source_file": "guide.md"}));
    write_graph(&graph_path, &data);
    graph_path
}

#[test]
fn rebuild_code_semantic_doc_not_double_represented_on_full_rebuild() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    let graph_path = seed_semantic_doc_graph(corpus);
    let before = read_graph(&graph_path);
    let before_count = before["nodes"].as_array().expect("nodes").len();

    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("rebuild"));

    let after = read_graph(&graph_path);
    let ids = node_ids(&after);
    for id in SEMANTIC_GUIDE_IDS {
        assert!(ids.contains(id), "semantic node {id} must be preserved");
    }
    for id in AST_GUIDE_IDS {
        assert!(
            !ids.contains(id),
            "AST heading node {id} minted for a semantic doc (#1915)"
        );
    }
    let after_count = after["nodes"].as_array().expect("nodes").len();
    assert_eq!(after_count, before_count, "node count inflated (#1915)");
}

fn incremental_preserves_semantic_doc(changed: &[PathBuf]) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    let graph_path = seed_semantic_doc_graph(corpus);

    assert!(rebuild_code(corpus, Some(changed), no_cluster_opts()).expect("incremental rebuild"));

    let after = read_graph(&graph_path);
    let ids = node_ids(&after);
    for id in SEMANTIC_GUIDE_IDS {
        assert!(
            ids.contains(id),
            "semantic node {id} wiped by incremental rebuild"
        );
    }
    let relations: std::collections::HashSet<(String, String, String)> = after["links"]
        .as_array()
        .expect("links")
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                e.get("target")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                e.get("relation")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect();
    assert!(
        relations.contains(&("guide_doc".into(), "auth_flow".into(), "explains".into())),
        "semantic doc edge dropped by incremental rebuild"
    );
    assert!(
        relations
            .iter()
            .any(|(s, _, r)| s == "auth_flow" && r == "implemented_by"),
        "doc-to-code semantic edge dropped by incremental rebuild"
    );
    for id in AST_GUIDE_IDS {
        assert!(
            !ids.contains(id),
            "incremental AST-scanned a semantic-backed doc (#1915)"
        );
    }
}

#[test]
fn rebuild_code_incremental_preserves_semantic_doc_nodes_and_edges_doc_only() {
    incremental_preserves_semantic_doc(&[PathBuf::from("guide.md")]);
}

#[test]
fn rebuild_code_incremental_preserves_semantic_doc_nodes_and_edges_doc_plus_code() {
    incremental_preserves_semantic_doc(&[PathBuf::from("guide.md"), PathBuf::from("app.py")]);
}

#[test]
fn rebuild_code_quick_scans_doc_without_semantic_nodes() {
    // #09b33b7 guard: a doc with NO semantic layer still gets the AST quick-scan
    // so no-LLM corpora keep their heading structure — #1915 must not regress it.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(corpus.join("app.py"), "def f():\n    return 1\n").expect("write");
    fs::write(corpus.join("notes.md"), "# Alpha\n\n## Beta\n").expect("write");
    let graph_path = corpus.join("graphify-out").join("graph.json");

    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("first rebuild"));
    let ids = node_ids(&read_graph(&graph_path));
    for id in ["notes", "notes_alpha", "notes_beta"] {
        assert!(
            ids.contains(id),
            "doc without semantic layer must be quick-scanned: {id}"
        );
    }

    // A rebuild over the existing graph (still no semantic nodes) keeps scanning.
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("second rebuild"));
    let ids = node_ids(&read_graph(&graph_path));
    for id in ["notes", "notes_alpha", "notes_beta"] {
        assert!(
            ids.contains(id),
            "quick-scan structure dropped on rebuild: {id}"
        );
    }
}

#[test]
fn rebuild_code_polluted_graph_self_heals_on_full_rebuild() {
    use serde_json::json;
    // #1915: a graph already bloated (semantic doc nodes PLUS stale `_origin=ast`
    // heading nodes for the same doc) sheds the heading nodes on the next full
    // rebuild via the AST-ownership rule — and the shrink guard accepts the
    // smaller write without force.
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path();
    fs::write(corpus.join("app.py"), "def handle_login():\n    return 1\n").expect("write");
    fs::write(
        corpus.join("guide.md"),
        "# Overview\n\n## Setup\n\n## Usage\n",
    )
    .expect("write");
    let graph_path = corpus.join("graphify-out").join("graph.json");
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("first rebuild"));
    let ids = node_ids(&read_graph(&graph_path));
    for id in AST_GUIDE_IDS {
        assert!(ids.contains(id), "initial quick-scan must mint {id}");
    }

    // Layer the semantic representation on top → the double-represented state.
    let mut data = read_graph(&graph_path);
    let nodes = data["nodes"].as_array_mut().expect("nodes");
    nodes.push(json!({"id": "guide_doc", "label": "Guide", "file_type": "document", "source_file": "guide.md"}));
    nodes.push(json!({"id": "auth_flow", "label": "Auth Flow", "file_type": "concept", "source_file": "guide.md"}));
    data["links"].as_array_mut().expect("links").push(json!({"source": "guide_doc", "target": "auth_flow", "relation": "explains", "confidence": "INFERRED", "source_file": "guide.md"}));
    write_graph(&graph_path, &data);

    // No force: the self-heal shrink must be accepted by the guard.
    assert!(rebuild_code(corpus, None, no_cluster_opts()).expect("self-heal rebuild"));
    let ids = node_ids(&read_graph(&graph_path));
    assert!(
        ids.contains("guide_doc") && ids.contains("auth_flow"),
        "semantic nodes preserved"
    );
    for id in AST_GUIDE_IDS {
        assert!(
            !ids.contains(id),
            "stale AST heading node {id} must be shed (#1915)"
        );
    }
}
