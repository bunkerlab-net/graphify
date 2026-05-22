//! Parity tests for manifest persistence and incremental detection.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — manifest / incremental tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use graphify_detect::{
    manifest::{detect_incremental_with_manifest, load_manifest_from_path, save_manifest_to_path},
    walk::detect,
};
use indexmap::IndexMap;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn save_and_load_manifest_roundtrip() {
    let tmp = tempdir().unwrap();
    let py = tmp.path().join("main.py");
    std::fs::write(&py, "print('hello')").unwrap();

    let manifest_path = tmp.path().join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert("code".to_string(), vec![py.to_str().unwrap().to_string()]);

    save_manifest_to_path(&files, &manifest_path, "both").unwrap();

    let manifest = load_manifest_from_path(&manifest_path).unwrap();
    assert!(manifest.contains_key(py.to_str().unwrap()));
    let entry = &manifest[py.to_str().unwrap()];
    assert!(!entry.ast_hash.is_empty(), "ast_hash must be set");
    assert!(!entry.semantic_hash.is_empty(), "semantic_hash must be set");
}

#[test]
fn load_manifest_returns_empty_for_missing_file() {
    let manifest_path = Path::new("/nonexistent/path/manifest.json");
    let result = load_manifest_from_path(manifest_path).unwrap();
    assert!(result.is_empty());
}

#[test]
fn save_manifest_code_file_stamped() {
    // Code files must be stamped in the manifest regardless of semantic cache.
    let tmp = tempdir().unwrap();
    let py = tmp.path().join("main.py");
    std::fs::write(&py, "print('hello')").unwrap();
    let manifest_path = tmp.path().join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert("code".to_string(), vec![py.to_str().unwrap().to_string()]);
    save_manifest_to_path(&files, &manifest_path, "both").unwrap();

    let manifest = load_manifest_from_path(&manifest_path).unwrap();
    let key = py.to_str().unwrap();
    assert!(manifest.contains_key(key));
    assert!(!manifest[key].ast_hash.is_empty());
}

#[test]
fn detect_incremental_all_new_when_no_manifest() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "x = 1").unwrap();
    let manifest_path = tmp.path().join("manifest.json");
    // No manifest on disk → everything is new
    let (changed, deleted, _updated) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .unwrap();
    assert!(!changed.is_empty());
    assert!(deleted.is_empty());
}

#[test]
fn detect_incremental_nothing_changed_after_save() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "x = 1").unwrap();
    std::fs::write(tmp.path().join("notes.md"), "# Notes\n\nSome content here.").unwrap();

    // First: detect and save manifest inside graphify-out/ so detect() skips it.
    let full = detect(tmp.path(), None, None);
    let gout = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&gout).unwrap();
    let manifest_path = gout.join("manifest.json");
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").unwrap();

    // Second: incremental should see no changes
    let (changed, deleted, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .unwrap();
    assert!(changed.is_empty(), "nothing changed, but got: {changed:?}");
    assert!(deleted.is_empty());
}

#[test]
fn detect_incremental_detects_new_file() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("main.py"), "x = 1").unwrap();
    let manifest_path = tmp.path().join("manifest.json");

    // Save initial manifest
    let full = detect(tmp.path(), None, None);
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").unwrap();

    // Add a new file
    std::fs::write(tmp.path().join("new.py"), "y = 2").unwrap();

    let (changed, _deleted, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .unwrap();
    assert!(
        changed
            .iter()
            .any(|p| p.to_str().unwrap().contains("new.py")),
        "new file must appear as changed"
    );
}

#[test]
fn detect_incremental_detects_deleted_file() {
    let tmp = tempdir().unwrap();
    let f1 = tmp.path().join("a.py");
    let f2 = tmp.path().join("b.py");
    std::fs::write(&f1, "x = 1").unwrap();
    std::fs::write(&f2, "y = 2").unwrap();
    let manifest_path = tmp.path().join("manifest.json");

    let full = detect(tmp.path(), None, None);
    let files: IndexMap<String, Vec<String>> = full.files.into_iter().collect();
    save_manifest_to_path(&files, &manifest_path, "both").unwrap();

    // Delete b.py
    std::fs::remove_file(&f2).unwrap();

    let (_changed, deleted, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None)
            .unwrap();
    assert!(
        deleted.iter().any(|p| p.to_str().unwrap().contains("b.py")),
        "deleted file must appear in deleted list"
    );
}

#[test]
fn detect_incremental_propagates_follow_symlinks() {
    // detect_incremental must forward follow_symlinks so symlinked sub-trees
    // appear in incremental scans the same way they appear in full scans.
    let tmp = tempdir().unwrap();
    let real_dir = tmp.path().join("real_corpus");
    std::fs::create_dir_all(&real_dir).unwrap();
    std::fs::write(real_dir.join("note.md"), "# real note\n\nsome content").unwrap();
    std::os::unix::fs::symlink(&real_dir, tmp.path().join("linked_corpus")).unwrap();

    let manifest_path = tmp.path().join("graphify-out").join("manifest.json");
    std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();

    // Without following symlinks, the symlinked dir contents are invisible.
    let (changed_no, _, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, Some(false), "semantic", None)
            .unwrap();
    assert!(
        !changed_no
            .iter()
            .any(|p| p.to_str().unwrap().contains("linked_corpus")),
        "symlinked contents must be invisible when follow_symlinks=false"
    );

    // With follow_symlinks=true, the symlinked dir contents appear and are new.
    let (changed_yes, _, updated) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, Some(true), "semantic", None)
            .unwrap();
    assert!(
        changed_yes
            .iter()
            .any(|p| p.to_str().unwrap().contains("linked_corpus")),
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
    save_manifest_to_path(&files, &manifest_path, "both").unwrap();
    let _ = updated; // suppress unused warning

    let (changed_second, _, _) =
        detect_incremental_with_manifest(tmp.path(), &manifest_path, Some(true), "semantic", None)
            .unwrap();
    assert_eq!(
        changed_second.len(),
        0,
        "no changes after saving manifest, but got: {changed_second:?}"
    );
}
