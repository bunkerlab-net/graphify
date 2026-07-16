//! Parity tests for manifest persistence and incremental detection.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — manifest / incremental tests.
#![allow(clippy::expect_used)]

use graphify_cache::{_reset_stat_index_for_tests, flush_stat_index};
use graphify_detect::{
    Manifest, detect_incremental, detect_incremental_with_cache_root,
    manifest::{
        detect_incremental_with_manifest, load_manifest_from_path,
        load_manifest_from_path_with_root, save_manifest_to_path, save_manifest_to_path_with_root,
    },
    save_manifest,
    walk::detect,
};
use indexmap::IndexMap;
use serial_test::serial;
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
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None, None)
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
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None, None)
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
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None, None)
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
        detect_incremental_with_manifest(tmp.path(), &manifest_path, None, "semantic", None, None)
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
    let (changed_no, _, _) = detect_incremental_with_manifest(
        tmp.path(),
        &manifest_path,
        Some(false),
        "semantic",
        None,
        None,
    )
    .expect("test invariant");
    assert!(
        !changed_no
            .iter()
            .any(|p| p.to_str().expect("utf-8 path").contains("linked_corpus")),
        "symlinked contents must be invisible when follow_symlinks=false"
    );

    // With follow_symlinks=true, the symlinked dir contents appear and are new.
    let (changed_yes, _, updated) = detect_incremental_with_manifest(
        tmp.path(),
        &manifest_path,
        Some(true),
        "semantic",
        None,
        None,
    )
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

    let (changed_second, _, _) = detect_incremental_with_manifest(
        tmp.path(),
        &manifest_path,
        Some(true),
        "semantic",
        None,
        None,
    )
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
fn detect_incremental_new_total_counts_changed_files_only() {
    let tmp = tempdir().expect("tempdir");
    // Canonicalize so manifest paths match detect()'s symlink-resolved root.
    let base = tmp.path().canonicalize().expect("canon");
    std::fs::write(base.join("c.py"), "z = 3").expect("test invariant");
    std::fs::write(base.join("d.py"), "w = 4").expect("test invariant");

    // Stamp both files into the manifest detect_incremental reads.
    let manifest = base.join("graphify-out").join("manifest.json");
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    files.insert(
        "code".to_string(),
        vec![
            base.join("c.py").to_string_lossy().into_owned(),
            base.join("d.py").to_string_lossy().into_owned(),
        ],
    );
    save_manifest_to_path_with_root(&files, &manifest, "both", Some(&base)).expect("save");

    // Change only c.py; d.py stays byte-identical.
    std::fs::write(base.join("c.py"), "z = 999").expect("test invariant");

    let inc = detect_incremental(&base, &Manifest::new()).expect("incremental");
    let changed: u64 = inc.changed_files.values().map(|v| v.len() as u64).sum();
    let unchanged: u64 = inc.unchanged_files.values().map(|v| v.len() as u64).sum();
    assert_eq!(changed, 1, "only c.py changed");
    assert_eq!(unchanged, 1, "d.py unchanged");
    // Python parity: new_total is the count of CHANGED files only, never the
    // total (changed + unchanged) file count.
    assert_eq!(
        inc.new_total, 1,
        "new_total must count changed files only, not changed + unchanged"
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
    assert_eq!(inc.new_total, 0);
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

/// #1747 / root-keyed stat index: two `detect_incremental_with_cache_root`
/// runs with DIFFERENT cache roots must persist their word-count / stat-index
/// cache to their OWN `--out` root. Before the fix the process-global index
/// ignored every root after the first, so the second corpus's entries were
/// written into the first root's index (a clobbered, shared cache).
#[test]
#[serial]
fn incremental_cache_root_is_per_invocation() {
    _reset_stat_index_for_tests();
    let corpus_a = tempdir().expect("tempdir");
    let corpus_b = tempdir().expect("tempdir");
    let root_a = tempdir().expect("tempdir");
    let root_b = tempdir().expect("tempdir");
    std::fs::write(corpus_a.path().join("alpha.txt"), "one two three").expect("test invariant");
    std::fs::write(corpus_b.path().join("beta.txt"), "four five six").expect("test invariant");

    // Two first-run detections (empty manifest), each with its own cache root.
    detect_incremental_with_cache_root(corpus_a.path(), &Manifest::new(), Some(root_a.path()))
        .expect("detect A");
    detect_incremental_with_cache_root(corpus_b.path(), &Manifest::new(), Some(root_b.path()))
        .expect("detect B");
    flush_stat_index().expect("flush");

    let idx = |root: &Path| {
        root.join("graphify-out")
            .join("cache")
            .join("stat-index.json")
    };
    let text_a = std::fs::read_to_string(idx(root_a.path())).expect("root A index must exist");
    let text_b = std::fs::read_to_string(idx(root_b.path())).expect("root B index must exist");

    // Each root caches ONLY its own corpus — no cross-contamination.
    assert!(text_a.contains("alpha.txt"), "root A must cache alpha.txt");
    assert!(
        !text_a.contains("beta.txt"),
        "root A must not hold beta.txt"
    );
    assert!(text_b.contains("beta.txt"), "root B must cache beta.txt");
    assert!(
        !text_b.contains("alpha.txt"),
        "root B must not hold alpha.txt"
    );
}

/// Read a file's mtime the same way `manifest::file_mtime` does, so a stored
/// value compares bit-equal to the one change detection reads back.
#[allow(clippy::cast_precision_loss)]
fn file_mtime_secs(p: &Path) -> f64 {
    let meta = std::fs::metadata(p).expect("metadata");
    let d = meta
        .modified()
        .expect("modified")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch");
    d.as_secs() as f64 + f64::from(d.subsec_nanos()) / 1_000_000_000.0
}

/// Write a legacy bare-float manifest (`{path: mtime}`) at the standard
/// `graphify-out/manifest.json` location so `detect_incremental` loads it.
fn write_legacy_float_manifest(root: &Path, src: &Path, mtime: f64) {
    let canon = std::fs::canonicalize(src).expect("canonicalize");
    let mut map = serde_json::Map::new();
    map.insert(
        canon.to_string_lossy().into_owned(),
        serde_json::json!(mtime),
    );
    let out = root.join("graphify-out");
    std::fs::create_dir_all(&out).expect("create_dir_all");
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_string(&serde_json::Value::Object(map)).expect("serialize"),
    )
    .expect("write manifest");
}

#[test]
#[serial]
fn detect_incremental_legacy_float_reextracts_on_backwards_mtime() {
    // #1859: a legacy float manifest must re-extract when mtime moves BACKWARDS
    // (git checkout of an older commit, tar/rsync restore). Pre-fix used `>`,
    // which silently kept the stale cache.
    _reset_stat_index_for_tests();
    let tmp = tempdir().expect("tempdir");
    let src = tmp.path().join("mod.py");
    std::fs::write(&src, "def old_content():\n    return 1\n").expect("write fixture");
    // Store a mtime FROM THE FUTURE, simulating a checkout of an older revision
    // that restored the file to an earlier timestamp.
    let future = file_mtime_secs(&src) + 3600.0;
    write_legacy_float_manifest(tmp.path(), &src, future);

    let result = detect_incremental(tmp.path(), &Manifest::new()).expect("incremental");
    let changed: Vec<String> = result.changed_files.values().flatten().cloned().collect();
    let unchanged: Vec<String> = result.unchanged_files.values().flatten().cloned().collect();
    assert!(
        changed.iter().any(|f| f.contains("mod.py")),
        "backwards-moving mtime on a legacy entry must re-extract: {changed:?}"
    );
    assert!(!unchanged.iter().any(|f| f.contains("mod.py")));
}

#[test]
#[serial]
fn detect_incremental_legacy_float_skips_when_mtime_matches() {
    // Non-regression: legacy float branch still skips when the stored mtime
    // equals the current mtime.
    _reset_stat_index_for_tests();
    let tmp = tempdir().expect("tempdir");
    let src = tmp.path().join("mod.py");
    std::fs::write(&src, "def stable():\n    return 1\n").expect("write fixture");
    let current = file_mtime_secs(&src);
    write_legacy_float_manifest(tmp.path(), &src, current);

    let result = detect_incremental(tmp.path(), &Manifest::new()).expect("incremental");
    let changed: Vec<String> = result.changed_files.values().flatten().cloned().collect();
    let unchanged: Vec<String> = result.unchanged_files.values().flatten().cloned().collect();
    assert!(
        !changed.iter().any(|f| f.contains("mod.py")),
        "exact match must skip: {changed:?}"
    );
    assert!(unchanged.iter().any(|f| f.contains("mod.py")));
}

#[test]
fn legacy_float_manifest_mtime_parses_bit_exact() {
    // Deterministic guard for serde_json's `float_roundtrip` feature, which the
    // legacy-float skip compare in `manifest_entry_changed` depends on. The
    // decimal `1784222299.0276783` is a real captured mtime whose nearest f64 is
    // 0x41da_9644_96c1_c57b; serde_json's DEFAULT parser mis-rounds it to
    // ...c57c (1 ULP off), which made `detect_incremental_legacy_float_*` flaky
    // (~13% of runs, timestamp-dependent). Loading a bare-float manifest must
    // reproduce the exact bits. Drop `float_roundtrip` and this fails every run.
    let tmp = tempdir().expect("tempdir");
    let path = tmp.path().join("manifest.json");
    std::fs::write(&path, r#"{"/x/mod.py": 1784222299.0276783}"#).expect("write manifest");
    let manifest = load_manifest_from_path(&path).expect("load manifest");
    let entry = manifest.get("/x/mod.py").expect("entry present");
    assert_eq!(
        entry.mtime.to_bits(),
        0x41da_9644_96c1_c57b,
        "serde_json must parse f64 correctly-rounded (float_roundtrip feature)"
    );
}
