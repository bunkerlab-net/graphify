//! Parity tests against `graphify-py/tests/test_cache.py`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use graphify_cache::{
    _reset_stat_index_for_tests, body_content, cache_dir, cached_files, clear_cache, file_hash,
    load_cached, save_cached,
};
use serde_json::json;
use serial_test::serial;

fn write_text(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write");
}

#[test]
#[serial]
fn file_hash_consistent() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("sample.txt");
    write_text(&f, "hello world");
    let h1 = file_hash(&f, tmp.path()).expect("hash");
    let h2 = file_hash(&f, tmp.path()).expect("hash");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64);
}

#[test]
#[serial]
fn file_hash_changes() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f1 = tmp.path().join("a.txt");
    let f2 = tmp.path().join("b.txt");
    write_text(&f1, "content one");
    write_text(&f2, "content two");
    let h1 = file_hash(&f1, tmp.path()).expect("hash");
    let h2 = file_hash(&f2, tmp.path()).expect("hash");
    assert_ne!(h1, h2);
}

#[test]
#[serial]
fn cache_roundtrip() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("sample.txt");
    write_text(&f, "hello world");
    let result = json!({
        "nodes": [{"id": "n1", "label": "Node1"}],
        "edges": [],
    });
    save_cached(&f, &result, tmp.path(), "ast").expect("save");
    let loaded = load_cached(&f, tmp.path(), "ast").expect("loaded");
    assert_eq!(loaded, result);
}

#[test]
#[serial]
fn cache_miss_on_change() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("sample.txt");
    write_text(&f, "hello world");
    let result = json!({"nodes": [], "edges": [{"source": "a", "target": "b"}]});
    save_cached(&f, &result, tmp.path(), "ast").expect("save");
    _reset_stat_index_for_tests(); // bust stat fastpath so we re-hash
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_text(&f, "completely different content");
    assert!(load_cached(&f, tmp.path(), "ast").is_none());
}

#[test]
#[serial]
fn cached_files_returns_hashes() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f1 = tmp.path().join("file1.py");
    let f2 = tmp.path().join("file2.py");
    write_text(&f1, "alpha");
    write_text(&f2, "beta");

    save_cached(&f1, &json!({"nodes": [], "edges": []}), tmp.path(), "ast").expect("save1");
    save_cached(&f2, &json!({"nodes": [], "edges": []}), tmp.path(), "ast").expect("save2");

    let hashes = cached_files(tmp.path());
    let h1 = file_hash(&f1, tmp.path()).expect("h1");
    let h2 = file_hash(&f2, tmp.path()).expect("h2");
    assert!(hashes.contains(&h1));
    assert!(hashes.contains(&h2));
}

#[test]
#[serial]
fn clear_cache_removes_all() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("sample.txt");
    write_text(&f, "hello world");
    save_cached(&f, &json!({"nodes": [], "edges": []}), tmp.path(), "ast").expect("save");
    let base = tmp.path().join("graphify-out").join("cache");
    let pre: Vec<_> = walkdir::WalkDir::new(&base)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(!pre.is_empty(), "expected pre-clear cache files");
    clear_cache(tmp.path()).expect("clear");
    let post: Vec<_> = walkdir::WalkDir::new(&base)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(post.is_empty(), "expected no cache files after clear");
}

#[test]
#[serial]
fn md_frontmatter_only_change_same_hash() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(
        &f,
        "---\nreviewed: 2026-01-01\n---\n\n# Title\n\nBody text.",
    );
    let h1 = file_hash(&f, tmp.path()).expect("h1");
    _reset_stat_index_for_tests(); // bust stat fastpath
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_text(
        &f,
        "---\nreviewed: 2026-04-09\n---\n\n# Title\n\nBody text.",
    );
    let h2 = file_hash(&f, tmp.path()).expect("h2");
    assert_eq!(h1, h2);
}

#[test]
#[serial]
fn md_body_change_different_hash() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(
        &f,
        "---\nreviewed: 2026-01-01\n---\n\n# Title\n\nOriginal body.",
    );
    let h1 = file_hash(&f, tmp.path()).expect("h1");
    _reset_stat_index_for_tests();
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_text(
        &f,
        "---\nreviewed: 2026-01-01\n---\n\n# Title\n\nChanged body.",
    );
    let h2 = file_hash(&f, tmp.path()).expect("h2");
    assert_ne!(h1, h2);
}

#[test]
#[serial]
fn md_no_frontmatter_hashed_normally() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Just a heading\n\nNo frontmatter here.");
    let h1 = file_hash(&f, tmp.path()).expect("h1");
    _reset_stat_index_for_tests();
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_text(&f, "# Just a heading\n\nDifferent content.");
    let h2 = file_hash(&f, tmp.path()).expect("h2");
    assert_ne!(h1, h2);
}

#[test]
#[serial]
fn non_md_file_hashed_fully() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("script.py");
    write_text(&f, "# comment\nx = 1");
    let h1 = file_hash(&f, tmp.path()).expect("h1");
    _reset_stat_index_for_tests();
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_text(&f, "# changed comment\nx = 1");
    let h2 = file_hash(&f, tmp.path()).expect("h2");
    assert_ne!(h1, h2);
}

#[test]
fn body_content_strips_frontmatter() {
    let content = b"---\ntitle: Test\n---\n\nActual body.";
    assert_eq!(body_content(content), b"\n\nActual body.");
}

#[test]
fn body_content_no_frontmatter() {
    let content = b"No frontmatter here.";
    assert_eq!(body_content(content), content);
}

#[test]
#[serial]
fn cache_dir_creates_kind_subdir() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = cache_dir(tmp.path(), "semantic").expect("cache_dir");
    assert!(dir.is_dir());
    assert!(dir.ends_with("semantic"));
}
