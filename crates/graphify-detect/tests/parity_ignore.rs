//! Parity tests for .graphifyignore / .gitignore matching.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — ignore pattern tests.
#![allow(clippy::expect_used)]

use graphify_detect::{is_ignored, load_graphifyignore};
use tempfile::tempdir;

#[test]
fn negation_cannot_rescue_file_under_excluded_dir() {
    let tmp = tempdir().expect("tempdir");
    let android = tmp.path().join("android").join("app").join("src");
    std::fs::create_dir_all(&android).expect("create_dir_all");
    let victim = android.join("Main.kt");
    std::fs::write(&victim, "fun main() {}").expect("test invariant");
    std::fs::write(tmp.path().join(".graphifyignore"), "android/\n!src/\n")
        .expect("test invariant");
    let patterns = load_graphifyignore(tmp.path());
    assert!(
        is_ignored(&victim, tmp.path(), &patterns),
        "android/app/src/Main.kt must remain ignored even with !src/ because \
         the parent android/ is excluded"
    );
}

#[test]
fn negation_works_when_no_ancestor_excluded() {
    let tmp = tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).expect("create_dir_all");
    let keep = src.join("keep.py");
    std::fs::write(&keep, "x = 1").expect("write fixture");
    std::fs::write(tmp.path().join(".graphifyignore"), "*.py\n!src/keep.py\n")
        .expect("test invariant");
    let patterns = load_graphifyignore(tmp.path());
    assert!(
        !is_ignored(&keep, tmp.path(), &patterns),
        "src/keep.py should be un-ignored by !src/keep.py since src/ itself is not excluded"
    );
}

#[test]
fn negation_ancestor_itself_reincluded() {
    let tmp = tempdir().expect("tempdir");
    let vendor = tmp.path().join("vendor").join("lib");
    std::fs::create_dir_all(&vendor).expect("create_dir_all");
    let f = vendor.join("utils.py");
    std::fs::write(&f, "x = 1").expect("write fixture");
    std::fs::write(tmp.path().join(".graphifyignore"), "vendor/\n!vendor/\n")
        .expect("test invariant");
    let patterns = load_graphifyignore(tmp.path());
    // vendor/ is excluded then re-included; ancestor eval returns False so file is evaluated on its own.
    assert!(!is_ignored(&f, tmp.path(), &patterns));
}

#[test]
fn graphifyignore_hermetic_without_vcs() {
    // Without a VCS root, parent .graphifyignore does NOT apply (hermetic).
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".graphifyignore"), "vendor/\n").expect("test invariant");
    let sub = tmp.path().join("packages").join("mylib");
    std::fs::create_dir_all(&sub).expect("create_dir_all");
    let vendor = sub.join("vendor");
    std::fs::create_dir_all(&vendor).expect("create_dir_all");
    let dep = vendor.join("dep.py");
    std::fs::write(&dep, "y = 2").expect("write fixture");

    let patterns = load_graphifyignore(&sub);
    // No .graphifyignore at or above `sub` (no VCS root links them)
    assert_eq!(patterns.len(), 0);
    assert!(!is_ignored(&dep, &sub, &patterns));
}

#[test]
fn graphifyignore_discovered_from_parent_in_vcs() {
    // Inside a VCS repo, parent .graphifyignore applies to subdir scans.
    let tmp = tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join(".git")).expect("test invariant");
    std::fs::write(tmp.path().join(".graphifyignore"), "vendor/\n").expect("test invariant");
    let sub = tmp.path().join("packages").join("mylib");
    std::fs::create_dir_all(&sub).expect("create_dir_all");
    let vendor_dir = sub.join("vendor");
    std::fs::create_dir_all(&vendor_dir).expect("create_dir_all");
    let dep = vendor_dir.join("dep.py");
    std::fs::write(&dep, "y = 2").expect("write fixture");

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
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".graphifyignore"), "main.py\n").expect("test invariant");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("test invariant");
    let sub = repo.join("sub");
    std::fs::create_dir_all(&sub).expect("create_dir_all");
    let main = sub.join("main.py");
    std::fs::write(&main, "x = 1").expect("write fixture");

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
    let tmp = tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("test invariant");
    std::fs::write(repo.join(".graphifyignore"), "vendor/\n").expect("test invariant");
    let sub = repo.join("packages").join("mylib");
    std::fs::create_dir_all(&sub).expect("create_dir_all");
    let vendor_dir = sub.join("vendor");
    std::fs::create_dir_all(&vendor_dir).expect("create_dir_all");
    let dep = vendor_dir.join("dep.py");
    std::fs::write(&dep, "y = 2").expect("write fixture");
    let main = sub.join("main.py");
    std::fs::write(&main, "x = 1").expect("write fixture");

    let patterns = load_graphifyignore(&sub);
    assert_eq!(patterns.len(), 1);
    assert!(!is_ignored(&main, &sub, &patterns));
    assert!(is_ignored(&dep, &sub, &patterns));
}

// ── #1087: anchored patterns must not leak a basename match into a subtree ─────
//
// `load_graphifyignore` canonicalises the root into the pattern anchors, so
// anchored matching strips that canonical anchor from the target. pytest's
// `tmp_path` is already resolved; mirror that by canonicalising here, otherwise
// the macOS `/var → /private/var` symlink defeats the anchored strip_prefix.

#[test]
fn anchored_dir_not_matched_at_depth() {
    // /inbox/ must NOT match src/inbox/ — only inbox/ at the anchor root.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let src_inbox = root.join("src").join("inbox");
    std::fs::create_dir_all(&src_inbox).expect("create_dir_all");
    let f = src_inbox.join("main.rs");
    std::fs::write(&f, "fn main() {}").expect("write fixture");
    std::fs::write(root.join(".graphifyignore"), "/inbox/\n").expect("write ignore");
    let patterns = load_graphifyignore(&root);
    assert!(
        !is_ignored(&f, &root, &patterns),
        "src/inbox/main.rs must NOT be ignored by /inbox/ — the pattern is anchored to root"
    );
    assert!(
        !is_ignored(&src_inbox, &root, &patterns),
        "src/inbox/ must NOT be ignored by /inbox/ — the pattern is anchored to root"
    );
}

#[test]
fn anchored_dir_matches_at_root() {
    // /inbox/ must still match inbox/ at the anchor root (positive case).
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let inbox = root.join("inbox");
    std::fs::create_dir_all(&inbox).expect("create_dir_all");
    let f = inbox.join("data.json");
    std::fs::write(&f, "{}").expect("write fixture");
    std::fs::write(root.join(".graphifyignore"), "/inbox/\n").expect("write ignore");
    let patterns = load_graphifyignore(&root);
    assert!(
        is_ignored(&f, &root, &patterns),
        "inbox/data.json must be ignored by /inbox/"
    );
    assert!(
        is_ignored(&inbox, &root, &patterns),
        "inbox/ must be ignored by /inbox/"
    );
}

#[test]
fn anchored_file_not_matched_at_depth() {
    // /build must not match src/build.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let src_build = root.join("src").join("build");
    std::fs::create_dir_all(&src_build).expect("create_dir_all");
    std::fs::write(root.join(".graphifyignore"), "/build\n").expect("write ignore");
    let patterns = load_graphifyignore(&root);
    assert!(
        !is_ignored(&src_build, &root, &patterns),
        "src/build must NOT be ignored by /build"
    );
}

#[test]
fn unanchored_dir_still_matches_at_depth() {
    // inbox/ (no leading /) must still match src/inbox/ anywhere in the tree.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let src_inbox = root.join("src").join("inbox");
    std::fs::create_dir_all(&src_inbox).expect("create_dir_all");
    let f = src_inbox.join("main.rs");
    std::fs::write(&f, "fn main() {}").expect("write fixture");
    std::fs::write(root.join(".graphifyignore"), "inbox/\n").expect("write ignore");
    let patterns = load_graphifyignore(&root);
    assert!(
        is_ignored(&f, &root, &patterns),
        "src/inbox/main.rs must be ignored by unanchored inbox/"
    );
}

#[test]
fn anchored_multi_segment_pattern() {
    // /src/inbox/ must match src/inbox/ but not x/src/inbox/.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    std::fs::create_dir_all(root.join("src").join("inbox")).expect("create_dir_all");
    std::fs::create_dir_all(root.join("x").join("src").join("inbox")).expect("create_dir_all");
    let target_ok = root.join("src").join("inbox").join("a.py");
    std::fs::write(&target_ok, "x=1").expect("write fixture");
    let target_bad = root.join("x").join("src").join("inbox").join("b.py");
    std::fs::write(&target_bad, "x=1").expect("write fixture");
    std::fs::write(root.join(".graphifyignore"), "/src/inbox/\n").expect("write ignore");
    let patterns = load_graphifyignore(&root);
    assert!(
        is_ignored(&target_ok, &root, &patterns),
        "src/inbox/a.py must be ignored by /src/inbox/"
    );
    assert!(
        !is_ignored(&target_bad, &root, &patterns),
        "x/src/inbox/b.py must NOT be ignored by /src/inbox/"
    );
}
