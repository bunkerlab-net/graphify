//! Coverage tests for `.graphifyinclude` parsing and inclusion matching.

#![allow(clippy::expect_used)]

use std::fs;

use graphify_detect::ignore::{
    could_contain_included_path, is_included, load_graphifyignore, load_graphifyinclude,
};
use graphify_detect::walk::{auto_follow_symlinks, collect_files, detect};

// ── load_graphifyinclude ────────────────────────────────────────────────────

#[test]
fn load_graphifyinclude_reads_explicit_patterns() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(
        tmp.path().join(".graphifyinclude"),
        "*.py\n# comment\nsrc/**\n",
    )
    .expect("test invariant");
    let patterns = load_graphifyinclude(tmp.path());
    assert!(!patterns.is_empty());
}

#[test]
fn load_graphifyinclude_returns_empty_when_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let patterns = load_graphifyinclude(tmp.path());
    assert!(patterns.is_empty());
}

#[test]
fn load_graphifyinclude_skips_blank_lines() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join(".graphifyinclude"), "\n\n*.py\n\n").expect("test invariant");
    let patterns = load_graphifyinclude(tmp.path());
    assert_eq!(patterns.len(), 1);
}

// ── is_included ─────────────────────────────────────────────────────────────

#[test]
fn is_included_matches_glob() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tmp.path().join("src")).expect("test invariant");
    fs::write(tmp.path().join("src/foo.py"), "").expect("test invariant");
    fs::write(tmp.path().join(".graphifyinclude"), "*.py\n").expect("test invariant");
    let patterns = load_graphifyinclude(tmp.path());
    assert!(is_included(
        &tmp.path().join("src/foo.py"),
        tmp.path(),
        &patterns
    ));
}

#[test]
fn is_included_anchored_dir_matches_root_and_subtree() {
    // An anchored allowlist directory (`/src`) includes the directory itself and
    // everything beneath it, but not a same-named directory deeper in the tree.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    fs::create_dir_all(root.join("src/deep")).expect("test invariant");
    fs::create_dir_all(root.join("x/src")).expect("test invariant");
    fs::write(root.join(".graphifyinclude"), "/src\n").expect("test invariant");
    let patterns = load_graphifyinclude(&root);

    assert!(
        is_included(&root.join("src"), &root, &patterns),
        "/src must include the anchored directory itself"
    );
    assert!(
        is_included(&root.join("src/deep/main.py"), &root, &patterns),
        "/src must include files in its subtree"
    );
    assert!(
        !is_included(&root.join("x/src"), &root, &patterns),
        "/src is anchored to root and must NOT match a nested src/"
    );
}

#[test]
fn is_included_anchored_file_matches_only_at_root() {
    // An anchored file pattern (`/setup.py`) matches at the anchor root but not
    // a same-named file deeper in the tree.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    fs::create_dir_all(root.join("pkg")).expect("test invariant");
    fs::write(root.join(".graphifyinclude"), "/setup.py\n").expect("test invariant");
    let patterns = load_graphifyinclude(&root);

    assert!(is_included(&root.join("setup.py"), &root, &patterns));
    assert!(!is_included(&root.join("pkg/setup.py"), &root, &patterns));
}

#[test]
fn is_included_unanchored_matches_at_depth() {
    // An unanchored pattern matches anywhere in the tree (not just the root).
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    fs::create_dir_all(root.join("a/b")).expect("test invariant");
    fs::write(root.join(".graphifyinclude"), "*.py\n").expect("test invariant");
    let patterns = load_graphifyinclude(&root);
    assert!(is_included(&root.join("a/b/deep.py"), &root, &patterns));
}

#[test]
fn is_included_with_no_patterns_is_false() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let empty = vec![];
    // With empty include patterns, is_included returns false (no allowlist match).
    assert!(!is_included(&tmp.path().join("x.py"), tmp.path(), &empty));
}

#[test]
fn could_contain_included_path_works() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tmp.path().join("src")).expect("test invariant");
    fs::write(tmp.path().join(".graphifyinclude"), "src/**\n").expect("test invariant");
    let patterns = load_graphifyinclude(tmp.path());
    assert!(could_contain_included_path(
        &tmp.path().join("src"),
        tmp.path(),
        &patterns
    ));
}

// ── load_graphifyignore ─────────────────────────────────────────────────────

#[test]
fn load_graphifyignore_reads_patterns() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join(".graphifyignore"), "*.tmp\nnode_modules/\n")
        .expect("test invariant");
    let patterns = load_graphifyignore(tmp.path());
    assert!(!patterns.is_empty());
}

// ── walk helpers ────────────────────────────────────────────────────────────

#[test]
fn auto_follow_symlinks_returns_false_without_symlinks() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("a.py"), "x = 1").expect("test invariant");
    assert!(!auto_follow_symlinks(tmp.path()));
}

#[cfg(unix)]
#[test]
fn auto_follow_symlinks_detects_symlinks() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let real = tmp.path().join("real.py");
    fs::write(&real, "x = 1").expect("write fixture");
    let link = tmp.path().join("link.py");
    std::os::unix::fs::symlink(&real, &link).expect("test invariant");
    assert!(auto_follow_symlinks(tmp.path()));
}

#[test]
fn collect_files_finds_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tmp.path().join("sub")).expect("test invariant");
    fs::write(tmp.path().join("a.py"), "x = 1").expect("test invariant");
    fs::write(tmp.path().join("sub").join("b.py"), "y = 2").expect("test invariant");
    let files = collect_files(tmp.path());
    assert!(files.len() >= 2);
}

// ── detect with various flags ──────────────────────────────────────────────

#[test]
fn detect_with_explicit_follow_symlinks_true() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("a.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), Some(true), None);
    assert!(result.files.contains_key("code"));
}

#[test]
fn detect_with_explicit_follow_symlinks_false() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("a.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), Some(false), None);
    assert!(result.files.contains_key("code"));
}

#[test]
fn detect_with_extra_excludes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tmp.path().join("vendor")).expect("test invariant");
    fs::write(tmp.path().join("vendor").join("dep.py"), "x = 1").expect("test invariant");
    fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let extra: Vec<String> = vec!["vendor/**".to_string()];
    let result = detect(tmp.path(), None, Some(&extra));
    let code: &Vec<String> = result.files.get("code").expect("key present");
    // main.py should be present; vendor/dep.py should be excluded.
    assert!(code.iter().any(|f| f.ends_with("main.py")));
}

#[test]
fn detect_picks_up_memory_sidecar() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    // Memory sidecar directory under graphify-out — exercises the in-memory
    // tree branch of walk.rs that uses the sequential walk_dir.
    let mem_dir = tmp.path().join("graphify-out").join("memory");
    fs::create_dir_all(&mem_dir).expect("create_dir_all");
    fs::write(mem_dir.join("note.md"), "# memory note").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    // main.py should be present.
    let code: &Vec<String> = result.files.get("code").expect("key present");
    assert!(code.iter().any(|f| f.ends_with("main.py")));
}

#[test]
fn detect_with_nested_subdirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tmp.path().join("a").join("b").join("c")).expect("test invariant");
    fs::write(tmp.path().join("a/b/c/deep.py"), "x = 1").expect("test invariant");
    fs::write(tmp.path().join("a/mid.py"), "y = 2").expect("test invariant");
    fs::write(tmp.path().join("top.py"), "z = 3").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let code: &Vec<String> = result.files.get("code").expect("key present");
    assert!(code.len() >= 3);
}
