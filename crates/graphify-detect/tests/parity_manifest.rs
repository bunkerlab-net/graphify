//! Parity tests for manifest persistence and incremental detection.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — manifest / incremental tests.
#![allow(clippy::expect_used)]

use graphify_detect::{
    Manifest, detect_incremental,
    manifest::{
        detect_incremental_with_manifest, load_manifest_from_path,
        load_manifest_from_path_with_root, save_manifest_to_path, save_manifest_to_path_with_root,
    },
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

// ── #777: portable manifest paths ───────────────────────────────────────────

/// Build the `kind -> [path]` files map the manifest writer expects.
fn files_map(pairs: &[(&str, &str)]) -> IndexMap<String, Vec<String>> {
    let mut m: IndexMap<String, Vec<String>> = IndexMap::new();
    for (kind, path) in pairs {
        m.entry((*kind).to_string())
            .or_default()
            .push((*path).to_string());
    }
    m
}

#[test]
fn save_manifest_relativizes_keys_when_root_given() {
    let tmp = tempdir().expect("tempdir");
    // Canonicalize so the test root matches the symlink-resolved form the
    // relativizer compares against (pytest's tmp_path is already resolved).
    let root_buf = tmp.path().canonicalize().expect("canon");
    let root = root_buf.as_path();
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src").join("foo.py"), "def x(): pass\n").expect("write");
    std::fs::write(root.join("doc.md"), "hello\n").expect("write");

    let manifest_path = root.join("graphify-out").join("manifest.json");
    let files = files_map(&[
        ("code", &root.join("src").join("foo.py").to_string_lossy()),
        ("document", &root.join("doc.md").to_string_lossy()),
    ]);
    save_manifest_to_path_with_root(&files, &manifest_path, "both", Some(root)).expect("save");

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read"))
            .expect("parse");
    let keys: std::collections::BTreeSet<&str> = raw
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["doc.md", "src/foo.py"].into_iter().collect(),
        "on-disk keys must be relative posix paths"
    );

    // Loaded with root → callers see absolute keys back (root is already
    // canonical, so a plain join reproduces the absolutized key).
    let loaded = load_manifest_from_path_with_root(&manifest_path, Some(root)).expect("load");
    let abs_foo = root
        .join("src")
        .join("foo.py")
        .to_string_lossy()
        .into_owned();
    let abs_doc = root.join("doc.md").to_string_lossy().into_owned();
    assert!(loaded.contains_key(&abs_foo));
    assert!(loaded.contains_key(&abs_doc));
}

#[test]
fn save_manifest_without_root_keeps_absolute_keys() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    let f = root.join("foo.py");
    std::fs::write(&f, "pass\n").expect("write");
    let manifest_path = root.join("graphify-out").join("manifest.json");
    let files = files_map(&[("code", &f.to_string_lossy())]);
    save_manifest_to_path(&files, &manifest_path, "both").expect("save");

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read"))
            .expect("parse");
    let key = raw
        .as_object()
        .expect("obj")
        .keys()
        .next()
        .expect("key")
        .clone();
    // Without root the key is stored verbatim (the absolute path passed in),
    // not relativized and not canonicalized.
    assert!(
        Path::new(&key).is_absolute(),
        "key must stay absolute, got {key}"
    );
    assert_eq!(key, f.to_string_lossy());
}

#[test]
fn load_manifest_absolutizes_relative_keys() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    let manifest_path = root.join("graphify-out").join("manifest.json");
    std::fs::create_dir_all(manifest_path.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &manifest_path,
        serde_json::to_string(&serde_json::json!({
            "src/foo.py": {"mtime": 0.0, "ast_hash": "h1", "semantic_hash": ""},
            "doc.md": {"mtime": 0.0, "ast_hash": "h2", "semantic_hash": ""},
        }))
        .expect("ser"),
    )
    .expect("write");

    let loaded = load_manifest_from_path_with_root(&manifest_path, Some(root)).expect("load");
    let root_resolved = root.canonicalize().expect("canon");
    assert!(
        loaded.contains_key(
            &root_resolved
                .join("src")
                .join("foo.py")
                .to_string_lossy()
                .into_owned()
        )
    );
    assert!(loaded.contains_key(&root_resolved.join("doc.md").to_string_lossy().into_owned()));
}

#[test]
fn load_manifest_passes_through_legacy_absolute_keys() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    let manifest_path = root.join("graphify-out").join("manifest.json");
    std::fs::create_dir_all(manifest_path.parent().expect("parent")).expect("mkdir");
    let abs_key = root
        .canonicalize()
        .expect("canon")
        .join("foo.py")
        .to_string_lossy()
        .into_owned();
    std::fs::write(
        &manifest_path,
        serde_json::to_string(&serde_json::json!({
            abs_key.clone(): {"mtime": 0.0, "ast_hash": "h", "semantic_hash": ""},
        }))
        .expect("ser"),
    )
    .expect("write");

    let loaded = load_manifest_from_path_with_root(&manifest_path, Some(root)).expect("load");
    assert!(loaded.contains_key(&abs_key));
}

#[test]
fn save_manifest_out_of_root_keeps_absolute() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("repo");
    std::fs::create_dir_all(&root).expect("mkdir");
    // A file outside `root` (sibling dir) must keep its absolute key.
    let outside = tmp.path().join("sibling.py");
    std::fs::write(&outside, "pass\n").expect("write");

    let manifest_path = root.join("graphify-out").join("manifest.json");
    let files = files_map(&[("code", &outside.to_string_lossy())]);
    save_manifest_to_path_with_root(&files, &manifest_path, "both", Some(&root)).expect("save");

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read"))
            .expect("parse");
    let key = raw.as_object().expect("obj").keys().next().expect("key");
    assert!(
        Path::new(key).is_absolute(),
        "out-of-root entries keep absolute keys, got {key}"
    );
}

#[test]
fn detect_incremental_portable_across_paths() {
    let tmp = tempdir().expect("tempdir");
    // Canonicalize the base so sub-roots are symlink-resolved (macOS /var).
    let base = tmp.path().canonicalize().expect("canon");
    // First "machine": create corpus, save manifest with root.
    let repo_a = base.join("repo_a");
    std::fs::create_dir_all(repo_a.join("src")).expect("mkdir a");
    std::fs::write(repo_a.join("src").join("foo.py"), "pass\n").expect("write");
    std::fs::write(repo_a.join("doc.md"), "hello\n").expect("write");
    let manifest_a = repo_a.join("graphify-out").join("manifest.json");
    let files = files_map(&[
        ("code", &repo_a.join("src").join("foo.py").to_string_lossy()),
        ("document", &repo_a.join("doc.md").to_string_lossy()),
    ]);
    save_manifest_to_path_with_root(&files, &manifest_a, "both", Some(&repo_a)).expect("save");

    // Second "machine": same corpus at a different absolute prefix + copied manifest.
    let repo_b = base.join("repo_b");
    std::fs::create_dir_all(repo_b.join("src")).expect("mkdir b");
    std::fs::write(repo_b.join("src").join("foo.py"), "pass\n").expect("write");
    std::fs::write(repo_b.join("doc.md"), "hello\n").expect("write");
    std::fs::create_dir_all(repo_b.join("graphify-out")).expect("mkdir out");
    std::fs::copy(
        &manifest_a,
        repo_b.join("graphify-out").join("manifest.json"),
    )
    .expect("copy");

    let inc = detect_incremental(&repo_b, &Manifest::new()).expect("incremental");
    assert_eq!(inc.new_total, 2);
    let changed: Vec<&String> = inc.changed_files.values().flatten().collect();
    assert!(
        changed.is_empty(),
        "manifest must port across absolute paths; changed={changed:?}"
    );
}

#[cfg(unix)]
#[test]
fn save_manifest_in_root_symlink_roundtrips() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("sub")).expect("mkdir");
    let target = root.join("sub").join("target.py");
    std::fs::write(&target, "pass\n").expect("write");
    let alias = root.join("alias.py");
    std::os::unix::fs::symlink(&target, &alias).expect("symlink");

    let manifest_path = root.join("graphify-out").join("manifest.json");
    // Use the resolved-root view of the alias so strip_prefix succeeds even when
    // the tempdir root itself contains a symlinked component (e.g. /tmp on macOS).
    let alias_key = root
        .canonicalize()
        .expect("canon")
        .join("alias.py")
        .to_string_lossy()
        .into_owned();
    let files = files_map(&[("code", &alias_key)]);
    save_manifest_to_path_with_root(&files, &manifest_path, "both", Some(root)).expect("save");

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read"))
            .expect("parse");
    let keys: Vec<&str> = raw
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys.contains(&"alias.py"),
        "symlink stored under own name, got {keys:?}"
    );
    assert!(
        !keys.contains(&"sub/target.py"),
        "must not store resolved target, got {keys:?}"
    );
}
