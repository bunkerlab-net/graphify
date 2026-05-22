//! Parity tests for .graphifyignore / .gitignore matching.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — ignore pattern tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use graphify_detect::{is_ignored, load_graphifyignore};
use tempfile::tempdir;

#[test]
fn negation_cannot_rescue_file_under_excluded_dir() {
    let tmp = tempdir().unwrap();
    let android = tmp.path().join("android").join("app").join("src");
    std::fs::create_dir_all(&android).unwrap();
    let victim = android.join("Main.kt");
    std::fs::write(&victim, "fun main() {}").unwrap();
    std::fs::write(tmp.path().join(".graphifyignore"), "android/\n!src/\n").unwrap();
    let patterns = load_graphifyignore(tmp.path());
    assert!(
        is_ignored(&victim, tmp.path(), &patterns),
        "android/app/src/Main.kt must remain ignored even with !src/ because \
         the parent android/ is excluded"
    );
}

#[test]
fn negation_works_when_no_ancestor_excluded() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let keep = src.join("keep.py");
    std::fs::write(&keep, "x = 1").unwrap();
    std::fs::write(tmp.path().join(".graphifyignore"), "*.py\n!src/keep.py\n").unwrap();
    let patterns = load_graphifyignore(tmp.path());
    assert!(
        !is_ignored(&keep, tmp.path(), &patterns),
        "src/keep.py should be un-ignored by !src/keep.py since src/ itself is not excluded"
    );
}

#[test]
fn negation_ancestor_itself_reincluded() {
    let tmp = tempdir().unwrap();
    let vendor = tmp.path().join("vendor").join("lib");
    std::fs::create_dir_all(&vendor).unwrap();
    let f = vendor.join("utils.py");
    std::fs::write(&f, "x = 1").unwrap();
    std::fs::write(tmp.path().join(".graphifyignore"), "vendor/\n!vendor/\n").unwrap();
    let patterns = load_graphifyignore(tmp.path());
    // vendor/ is excluded then re-included; ancestor eval returns False so file is evaluated on its own.
    assert!(!is_ignored(&f, tmp.path(), &patterns));
}

#[test]
fn graphifyignore_hermetic_without_vcs() {
    // Without a VCS root, parent .graphifyignore does NOT apply (hermetic).
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join(".graphifyignore"), "vendor/\n").unwrap();
    let sub = tmp.path().join("packages").join("mylib");
    std::fs::create_dir_all(&sub).unwrap();
    let vendor = sub.join("vendor");
    std::fs::create_dir_all(&vendor).unwrap();
    let dep = vendor.join("dep.py");
    std::fs::write(&dep, "y = 2").unwrap();

    let patterns = load_graphifyignore(&sub);
    // No .graphifyignore at or above `sub` (no VCS root links them)
    assert_eq!(patterns.len(), 0);
    assert!(!is_ignored(&dep, &sub, &patterns));
}

#[test]
fn graphifyignore_discovered_from_parent_in_vcs() {
    // Inside a VCS repo, parent .graphifyignore applies to subdir scans.
    let tmp = tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::write(tmp.path().join(".graphifyignore"), "vendor/\n").unwrap();
    let sub = tmp.path().join("packages").join("mylib");
    std::fs::create_dir_all(&sub).unwrap();
    let vendor_dir = sub.join("vendor");
    std::fs::create_dir_all(&vendor_dir).unwrap();
    let dep = vendor_dir.join("dep.py");
    std::fs::write(&dep, "y = 2").unwrap();

    let patterns = load_graphifyignore(&sub);
    assert!(
        !patterns.is_empty(),
        "parent .graphifyignore must be picked up"
    );
    assert!(is_ignored(&dep, &sub, &patterns));
}

#[test]
fn graphifyignore_stops_at_git_boundary() {
    // Upward search stops at the git repo root.
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join(".graphifyignore"), "main.py\n").unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let sub = repo.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let main = sub.join("main.py");
    std::fs::write(&main, "x = 1").unwrap();

    let patterns = load_graphifyignore(&sub);
    assert_eq!(
        patterns.len(),
        0,
        "patterns from above repo root must not leak in"
    );
    assert!(!is_ignored(&main, &sub, &patterns));
}

#[test]
fn graphifyignore_at_git_root_is_included() {
    // A .graphifyignore at the git repo root is included when scanning a subdir.
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".graphifyignore"), "vendor/\n").unwrap();
    let sub = repo.join("packages").join("mylib");
    std::fs::create_dir_all(&sub).unwrap();
    let vendor_dir = sub.join("vendor");
    std::fs::create_dir_all(&vendor_dir).unwrap();
    let dep = vendor_dir.join("dep.py");
    std::fs::write(&dep, "y = 2").unwrap();
    let main = sub.join("main.py");
    std::fs::write(&main, "x = 1").unwrap();

    let patterns = load_graphifyignore(&sub);
    assert_eq!(patterns.len(), 1);
    assert!(!is_ignored(&main, &sub, &patterns));
    assert!(is_ignored(&dep, &sub, &patterns));
}
