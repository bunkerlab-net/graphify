//! #1774 — `extract()` must never write its AST cache into the analyzed source
//! tree. With no `cache_root` the cache location defaults to the current working
//! directory; an explicit `cache_root` still wins. The cache LOCATION is
//! decoupled from the key/id ANCHOR (the inferred common parent), so content
//! hashes stay relative and portable even for a corpus outside CWD.
//!
//! Ports `graphify-py/tests/test_extract_cache_location.py`. These tests `chdir`
//! and touch the process-global stat index, so they run `#[serial]`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use graphify_cache::{_reset_stat_index_for_tests, file_hash, flush_stat_index, load_cached};
use graphify_extract::extract;
use serial_test::serial;

/// RAII guard: restore the original working directory on drop.
struct CwdGuard(PathBuf);
impl CwdGuard {
    fn enter(dir: &Path) -> Self {
        let prev = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(dir).expect("chdir");
        Self(prev)
    }
}
impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

fn make_corpus(base: &Path) -> PathBuf {
    let corpus = base.join("corpus");
    std::fs::create_dir_all(&corpus).expect("mkdir corpus");
    std::fs::write(
        corpus.join("a.py"),
        "class Base:\n    def hello(self):\n        return 1\n",
    )
    .expect("write a.py");
    std::fs::write(
        corpus.join("b.py"),
        "from a import Base\n\nclass Sub(Base):\n    pass\n",
    )
    .expect("write b.py");
    corpus
}

#[test]
#[serial]
fn default_cache_lands_in_cwd_not_source_tree() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = make_corpus(tmp.path());
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("mkdir work");
    let _cwd = CwdGuard::enter(&work);

    let result = extract(&[corpus.join("a.py"), corpus.join("b.py")], None);

    assert!(
        !result.nodes.is_empty(),
        "extraction should still produce nodes"
    );
    assert!(
        !corpus.join("graphify-out").exists(),
        "cache/stat-index written into the analyzed source tree (#1774)"
    );
    assert!(
        work.join("graphify-out").join("cache").is_dir(),
        "cache should land under CWD"
    );
}

#[test]
#[serial]
fn default_cache_does_not_leave_stat_index_in_source_tree() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = make_corpus(tmp.path());
    let work = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&work).expect("mkdir work");
    let _cwd = CwdGuard::enter(&work);

    let _ = extract(&[corpus.join("a.py"), corpus.join("b.py")], None);
    // The stat index is buffered in memory; force a flush to assert WHERE it lands.
    flush_stat_index().expect("flush stat index");

    assert!(
        !corpus.join("graphify-out").exists(),
        "stat-index leaked into the corpus"
    );
    assert!(
        work.join("graphify-out")
            .join("cache")
            .join("stat-index.json")
            .exists(),
        "stat-index should be written under the cache location (CWD)"
    );
}

#[test]
#[serial]
fn explicit_cache_root_still_wins() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = make_corpus(tmp.path());
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("mkdir work");
    let out = tmp.path().join("out");
    let _cwd = CwdGuard::enter(&work);

    let _ = extract(&[corpus.join("a.py")], Some(&out));

    assert!(out.join("graphify-out").join("cache").is_dir());
    assert!(!corpus.join("graphify-out").exists());
    assert!(!work.join("graphify-out").exists());
}

#[test]
#[serial]
fn default_cache_round_trips_via_extract() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = make_corpus(tmp.path());
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("mkdir work");
    let _cwd = CwdGuard::enter(&work);

    let _ = extract(&[corpus.join("a.py")], None);
    // Look up with the anchor extract() uses (the corpus dir) and the CWD cache
    // location (".") — this must hit the entry the first run wrote.
    let root = corpus.canonicalize().expect("canonicalize corpus");
    let hit = load_cached(&corpus.join("a.py"), &root, "ast", Some(Path::new(".")));
    assert!(
        hit.is_some(),
        "second run should hit the CWD cache written by the first"
    );
}

#[test]
#[serial]
fn cache_keys_stay_relative_for_out_of_cwd_corpus() {
    use sha2::{Digest, Sha256};
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = make_corpus(tmp.path());
    let work = tmp.path().join("elsewhere").join("work");
    std::fs::create_dir_all(&work).expect("mkdir work");
    let _cwd = CwdGuard::enter(&work);

    let _ = extract(&[corpus.join("a.py")], None);

    let root = corpus.canonicalize().expect("canonicalize corpus");
    let key = file_hash(corpus.join("a.py"), &root, None).expect("file_hash");
    let raw = std::fs::read(corpus.join("a.py")).expect("read a.py");

    let key_with = |anchor_rel: &str| -> String {
        let mut h = Sha256::new();
        h.update(&raw);
        h.update([0u8]);
        h.update(anchor_rel.to_lowercase().as_bytes());
        hex::encode(h.finalize())
    };

    // Portable: keyed on the relative path within the corpus...
    assert_eq!(key, key_with("a.py"));
    // ...not on the absolute path (which a CWD-anchor one-liner would produce).
    let abs_rel = corpus
        .join("a.py")
        .canonicalize()
        .expect("canon a.py")
        .to_string_lossy()
        .into_owned();
    assert_ne!(key, key_with(&abs_rel));
}

#[test]
#[serial]
fn cjs_bypasses_ast_disk_cache() {
    // `.cjs` is in JS_CACHE_BYPASS_SUFFIXES (#1922): like `.js`/`.mjs`, its AST
    // result must NOT round-trip through the on-disk cache (sibling-resolution
    // staleness), whereas a `.py` file in the same run IS cached. Removing `cjs`
    // from the bypass set would make the negative assertion below fail.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus).expect("mkdir corpus");
    std::fs::write(
        corpus.join("main.cjs"),
        "function createWindow() {}\nmodule.exports = { createWindow };\n",
    )
    .expect("write main.cjs");
    std::fs::write(corpus.join("mod.py"), "def hello():\n    return 1\n").expect("write mod.py");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("mkdir work");
    let _cwd = CwdGuard::enter(&work);

    let result = extract(&[corpus.join("main.cjs"), corpus.join("mod.py")], None);
    assert!(
        result.nodes.iter().any(|n| {
            n.get("source_file")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| s.ends_with("main.cjs"))
                || n.get("label")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| s.contains("createWindow"))
        }),
        ".cjs must be extracted as JavaScript (#1922): {:?}",
        result
            .nodes
            .iter()
            .filter_map(|n| n.get("label").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
    );

    let root = corpus.canonicalize().expect("canonicalize corpus");
    assert!(
        load_cached(&corpus.join("mod.py"), &root, "ast", Some(Path::new("."))).is_some(),
        ".py should be written to the AST cache (positive control)"
    );
    assert!(
        load_cached(&corpus.join("main.cjs"), &root, "ast", Some(Path::new("."))).is_none(),
        ".cjs must bypass the AST disk cache (JS_CACHE_BYPASS_SUFFIXES)"
    );
}
