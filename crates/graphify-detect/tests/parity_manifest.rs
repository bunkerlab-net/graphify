//! Parity tests for manifest persistence and incremental detection.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — manifest / incremental tests.
#![allow(clippy::expect_used)]

use graphify_detect::{
    Manifest, detect_incremental,
    manifest::{detect_incremental_with_manifest, load_manifest_from_path, save_manifest_to_path},
    save_manifest,
    walk::detect,
};
use indexmap::IndexMap;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn save_and_load_manifest_roundtrip() {
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("main.py");
    std::fs::write(&py, "print('hello')").expect("test invariant");

    let manifest_path = tmp.path().join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert(
        "code".to_string(),
        vec![py.to_str().expect("utf-8 path").to_string()],
    );

    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");

    let manifest = load_manifest_from_path(&manifest_path).expect("test invariant");
    assert!(manifest.contains_key(py.to_str().expect("utf-8 path")));
    let entry = &manifest[py.to_str().expect("utf-8 path")];
    assert!(!entry.ast_hash.is_empty(), "ast_hash must be set");
    assert!(!entry.semantic_hash.is_empty(), "semantic_hash must be set");
}

#[test]
fn load_manifest_returns_empty_for_missing_file() {
    let manifest_path = Path::new("/nonexistent/path/manifest.json");
    let result = load_manifest_from_path(manifest_path).expect("test invariant");
    assert!(result.is_empty());
}

#[test]
fn save_manifest_code_file_stamped() {
    // Code files must be stamped in the manifest regardless of semantic cache.
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("main.py");
    std::fs::write(&py, "print('hello')").expect("test invariant");
    let manifest_path = tmp.path().join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert(
        "code".to_string(),
        vec![py.to_str().expect("utf-8 path").to_string()],
    );
    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");

    let manifest = load_manifest_from_path(&manifest_path).expect("test invariant");
    let key = py.to_str().expect("utf-8 path");
    assert!(manifest.contains_key(key));
    assert!(!manifest[key].ast_hash.is_empty());
}

#[test]
fn detect_incremental_all_new_when_no_manifest() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let manifest_path = tmp.path().join("manifest.json");
    // No manifest on disk → everything is new
    let (changed, deleted, _updated) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .expect("test invariant");
    assert!(!changed.is_empty());
    assert!(deleted.is_empty());
}

#[test]
fn detect_incremental_nothing_changed_after_save() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("notes.md"), "# Notes\n\nSome content here.")
        .expect("test invariant");

    // First: detect and save manifest inside graphify-out/ so detect() skips it.
    let full = detect(tmp.path(), None, None);
    let gout = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&gout).expect("create_dir_all");
    let manifest_path = gout.join("manifest.json");
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");

    // Second: incremental should see no changes
    let (changed, deleted, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .expect("test invariant");
    assert!(changed.is_empty(), "nothing changed, but got: {changed:?}");
    assert!(deleted.is_empty());
}

#[test]
fn detect_incremental_detects_new_file() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let manifest_path = tmp.path().join("manifest.json");

    // Save initial manifest
    let full = detect(tmp.path(), None, None);
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");

    // Add a new file
    std::fs::write(tmp.path().join("new.py"), "y = 2").expect("test invariant");

    let (changed, _deleted, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .expect("test invariant");
    assert!(
        changed
            .iter()
            .any(|p| p.to_str().expect("utf-8 path").contains("new.py")),
        "new file must appear as changed"
    );
}

#[test]
fn detect_incremental_detects_deleted_file() {
    let tmp = tempdir().expect("tempdir");
    let f1 = tmp.path().join("a.py");
    let f2 = tmp.path().join("b.py");
    std::fs::write(&f1, "x = 1").expect("write fixture");
    std::fs::write(&f2, "y = 2").expect("write fixture");
    let manifest_path = tmp.path().join("manifest.json");

    let full = detect(tmp.path(), None, None);
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");

    // Delete b.py
    std::fs::remove_file(&f2).expect("remove fixture");

    let (_changed, deleted, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .expect("test invariant");
    assert!(
        deleted
            .iter()
            .any(|p| p.to_str().expect("utf-8 path").contains("b.py")),
        "deleted file must appear in deleted list"
    );
}

#[cfg(unix)]
#[test]
fn detect_incremental_propagates_follow_symlinks() {
    // detect_incremental must forward follow_symlinks so symlinked sub-trees
    // appear in incremental scans the same way they appear in full scans.
    let tmp = tempdir().expect("tempdir");
    let real_dir = tmp.path().join("real_corpus");
    std::fs::create_dir_all(&real_dir).expect("create_dir_all");
    std::fs::write(real_dir.join("note.md"), "# real note\n\nsome content")
        .expect("test invariant");
    std::os::unix::fs::symlink(&real_dir, tmp.path().join("linked_corpus"))
        .expect("test invariant");

    let manifest_path = tmp.path().join("graphify-out").join("manifest.json");
    std::fs::create_dir_all(manifest_path.parent().expect("create_dir_all"))
        .expect("test invariant");

    // Without following symlinks, the symlinked dir contents are invisible.
    let (changed_no, _, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, Some(false), "semantic", None)
            .expect("test invariant");
    assert!(
        !changed_no
            .iter()
            .any(|p| p.to_str().expect("utf-8 path").contains("linked_corpus")),
        "symlinked contents must be invisible when follow_symlinks=false"
    );

    // With follow_symlinks=true, the symlinked dir contents appear and are new.
    let (changed_yes, _, updated) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, Some(true), "semantic", None)
            .expect("test invariant");
    assert!(
        changed_yes
            .iter()
            .any(|p| p.to_str().expect("utf-8 path").contains("linked_corpus")),
        "symlinked contents must appear when follow_symlinks=true"
    );
    assert!(
        changed_yes.len() >= 2,
        "real + linked files must both be new"
    );

    // After saving manifest with these files, a second incremental scan should see no changes.
    let files: IndexMap<String, Vec<String>> = {
        let full = detect(tmp.path(), Some(true), None);
        full.files.into_iter().collect()
    };
    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");
    let _ = updated; // suppress unused warning

    let (changed_second, _, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, Some(true), "semantic", None)
            .expect("test invariant");
    assert_eq!(
        changed_second.len(),
        0,
        "no changes after saving manifest, but got: {changed_second:?}"
    );
}

// ── Group 2A: save_manifest kind variants ────────────────────────────────────

#[test]
fn save_manifest_kind_ast_stamps_ast_hash_only() {
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("main.py");
    std::fs::write(&py, "x = 1").expect("write fixture");
    let manifest_path = tmp.path().join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert(
        "code".to_string(),
        vec![py.to_str().expect("utf-8 path").to_string()],
    );
    save_manifest(&files, &manifest_path, "ast").expect("test invariant");

    let manifest = load_manifest_from_path(&manifest_path).expect("test invariant");
    let entry = &manifest[py.to_str().expect("utf-8 path")];
    assert!(!entry.ast_hash.is_empty(), "ast_hash must be set");
    assert!(
        entry.semantic_hash.is_empty(),
        "semantic_hash must be empty for kind=ast"
    );
}

#[test]
fn save_manifest_kind_semantic_stamps_semantic_hash_only() {
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("main.py");
    std::fs::write(&py, "x = 1").expect("write fixture");
    let manifest_path = tmp.path().join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert(
        "code".to_string(),
        vec![py.to_str().expect("utf-8 path").to_string()],
    );
    save_manifest(&files, &manifest_path, "semantic").expect("test invariant");

    let manifest = load_manifest_from_path(&manifest_path).expect("test invariant");
    let entry = &manifest[py.to_str().expect("utf-8 path")];
    assert!(!entry.semantic_hash.is_empty(), "semantic_hash must be set");
    assert!(
        entry.ast_hash.is_empty(),
        "ast_hash must be empty for kind=semantic"
    );
}

#[test]
fn save_manifest_kind_both_stamps_both_hashes() {
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("main.py");
    std::fs::write(&py, "x = 1").expect("write fixture");
    let manifest_path = tmp.path().join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert(
        "code".to_string(),
        vec![py.to_str().expect("utf-8 path").to_string()],
    );
    save_manifest(&files, &manifest_path, "both").expect("test invariant");

    let manifest = load_manifest_from_path(&manifest_path).expect("test invariant");
    let entry = &manifest[py.to_str().expect("utf-8 path")];
    assert!(!entry.ast_hash.is_empty(), "ast_hash must be set");
    assert!(!entry.semantic_hash.is_empty(), "semantic_hash must be set");
}

// ── Group 2B: IncrementalDetectResult struct tests ───────────────────────────

#[test]
fn detect_incremental_returns_struct_with_incremental_true() {
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("a.py");
    std::fs::write(&py, "x = 1").expect("write fixture");
    // First: save a manifest so the second call is truly incremental.
    let gout = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&gout).expect("create_dir_all");
    let manifest_path = gout.join("manifest.json");
    let full = detect(tmp.path(), None, None);
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");

    let prev: Manifest = IndexMap::new();
    let result = detect_incremental(tmp.path(), &prev).expect("test invariant");
    assert!(
        result.incremental,
        "should be incremental when manifest exists"
    );
}

#[test]
fn detect_incremental_struct_changed_files_keyed_by_type() {
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("a.py");
    std::fs::write(&py, "x = 1").expect("write fixture");

    // First run with empty manifest → everything is new.
    let prev: Manifest = IndexMap::new();
    let result = detect_incremental(tmp.path(), &prev).expect("test invariant");
    // changed_files should have a "code" key (or at least some key) with the file.
    let all_changed: Vec<String> = result.changed_files.values().flatten().cloned().collect();
    assert!(
        all_changed.iter().any(|p| p.contains("a.py")),
        "a.py must appear in changed_files; got {all_changed:?}"
    );
    // Keys are file type strings (e.g. "code"), not paths.
    for k in result.changed_files.keys() {
        assert!(
            !k.contains('/') && !k.contains('.'),
            "changed_files key should be a file type, not a path: {k}"
        );
    }
}

#[test]
fn detect_incremental_struct_unchanged_files_keyed_by_type() {
    let tmp = tempdir().expect("tempdir");
    let py = tmp.path().join("b.py");
    std::fs::write(&py, "y = 2").expect("write fixture");

    // Save a manifest so the file is already known.
    let gout = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&gout).expect("create_dir_all");
    let manifest_path = gout.join("manifest.json");
    let full = detect(tmp.path(), None, None);
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").expect("test invariant");

    // Now incremental: b.py is unchanged.
    let prev: Manifest = IndexMap::new();
    let result = detect_incremental(tmp.path(), &prev).expect("test invariant");
    let all_unchanged: Vec<String> = result.unchanged_files.values().flatten().cloned().collect();
    assert!(
        all_unchanged.iter().any(|p| p.contains("b.py")),
        "b.py must appear in unchanged_files; got {all_unchanged:?}"
    );
}

#[test]
fn detect_incremental_struct_new_total_matches_changed_count() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("c.py"), "z = 3").expect("test invariant");
    std::fs::write(tmp.path().join("d.py"), "w = 4").expect("test invariant");

    let prev: Manifest = IndexMap::new();
    let result = detect_incremental(tmp.path(), &prev).expect("test invariant");
    let total_changed: u64 = result.changed_files.values().map(|v| v.len() as u64).sum();
    let total_unchanged: u64 = result
        .unchanged_files
        .values()
        .map(|v| v.len() as u64)
        .sum();
    assert_eq!(
        result.new_total,
        total_changed + total_unchanged,
        "new_total must equal changed + unchanged file counts"
    );
}
