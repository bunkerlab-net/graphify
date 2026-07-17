//! Parity tests against `graphify-py/tests/test_cache.py`.
#![allow(clippy::expect_used, unsafe_code)]

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use graphify_cache::{
    _reset_stat_index_for_tests, SemanticCacheOptions, StatIndexFlushGuard, body_content,
    cache_dir, cache_dir_versioned, cached_files, cached_word_count, check_semantic_cache,
    clear_cache, file_hash, flush_stat_index, load_cached, load_cached_versioned,
    prune_semantic_cache, remove_semantic_cache_entries, save_cached, save_cached_versioned,
    save_semantic_cache,
};
use serde_json::{Value, json};
use serial_test::serial;

fn write_text(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write");
}

/// Deterministically advance `path`'s mtime instead of sleeping, so the
/// stat-index fastpath sees a change on filesystems with coarse mtime
/// resolution (HFS+, FAT32, NFS).
fn bump_mtime(path: &Path) {
    let f = fs::OpenOptions::new().write(true).open(path).expect("open");
    let new_mtime = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    f.set_modified(new_mtime).expect("set_modified");
}

#[test]
#[serial]
fn file_hash_consistent() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("sample.txt");
    write_text(&f, "hello world");
    let h1 = file_hash(&f, tmp.path(), None).expect("hash");
    let h2 = file_hash(&f, tmp.path(), None).expect("hash");
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
    let h1 = file_hash(&f1, tmp.path(), None).expect("hash");
    let h2 = file_hash(&f2, tmp.path(), None).expect("hash");
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
    save_cached(&f, &result, tmp.path(), "ast", None).expect("save");
    let loaded = load_cached(&f, tmp.path(), "ast", None).expect("loaded");
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
    save_cached(&f, &result, tmp.path(), "ast", None).expect("save");
    _reset_stat_index_for_tests(); // bust stat fastpath so we re-hash
    write_text(&f, "completely different content");
    bump_mtime(&f);
    assert!(load_cached(&f, tmp.path(), "ast", None).is_none());
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

    save_cached(
        &f1,
        &json!({"nodes": [], "edges": []}),
        tmp.path(),
        "ast",
        None,
    )
    .expect("save1");
    save_cached(
        &f2,
        &json!({"nodes": [], "edges": []}),
        tmp.path(),
        "ast",
        None,
    )
    .expect("save2");

    let hashes = cached_files(tmp.path());
    let h1 = file_hash(&f1, tmp.path(), None).expect("h1");
    let h2 = file_hash(&f2, tmp.path(), None).expect("h2");
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
    save_cached(
        &f,
        &json!({"nodes": [], "edges": []}),
        tmp.path(),
        "ast",
        None,
    )
    .expect("save");
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
    let h1 = file_hash(&f, tmp.path(), None).expect("h1");
    _reset_stat_index_for_tests(); // bust stat fastpath
    write_text(
        &f,
        "---\nreviewed: 2026-04-09\n---\n\n# Title\n\nBody text.",
    );
    bump_mtime(&f);
    let h2 = file_hash(&f, tmp.path(), None).expect("h2");
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
    let h1 = file_hash(&f, tmp.path(), None).expect("h1");
    _reset_stat_index_for_tests();
    write_text(
        &f,
        "---\nreviewed: 2026-01-01\n---\n\n# Title\n\nChanged body.",
    );
    bump_mtime(&f);
    let h2 = file_hash(&f, tmp.path(), None).expect("h2");
    assert_ne!(h1, h2);
}

#[test]
#[serial]
fn md_no_frontmatter_hashed_normally() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Just a heading\n\nNo frontmatter here.");
    let h1 = file_hash(&f, tmp.path(), None).expect("h1");
    _reset_stat_index_for_tests();
    write_text(&f, "# Just a heading\n\nDifferent content.");
    bump_mtime(&f);
    let h2 = file_hash(&f, tmp.path(), None).expect("h2");
    assert_ne!(h1, h2);
}

#[test]
#[serial]
fn non_md_file_hashed_fully() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("script.py");
    write_text(&f, "# comment\nx = 1");
    let h1 = file_hash(&f, tmp.path(), None).expect("h1");
    _reset_stat_index_for_tests();
    write_text(&f, "# changed comment\nx = 1");
    bump_mtime(&f);
    let h2 = file_hash(&f, tmp.path(), None).expect("h2");
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

// ── StatIndexFlushGuard: flush on normal process exit (#1656) ─────────────────

#[test]
fn stat_index_flush_guard_persists_on_process_exit() {
    // The #1656 cache only helps if it survives to disk between runs. The guard
    // must flush when it drops at scope/process exit, with NO explicit
    // `flush_stat_index()` call. A real subprocess proves this: the prior
    // `static`-owned sentinel would never drop and so never write the index.
    const ROOT_ENV: &str = "GRAPHIFY_FLUSH_TEST_ROOT";
    const FILE_ENV: &str = "GRAPHIFY_FLUSH_TEST_FILE";

    if let (Ok(root), Ok(file)) = (std::env::var(ROOT_ENV), std::env::var(FILE_ENV)) {
        // CHILD: mutate the index, then return so the guard drops on exit.
        let _guard = StatIndexFlushGuard::new();
        let root = std::path::PathBuf::from(root);
        let file = std::path::PathBuf::from(&file);
        let parent = file.parent().unwrap_or_else(|| Path::new("."));
        assert_eq!(cached_word_count(&file, parent, |_| 7, Some(&root)), 7);
        return;
    }

    // PARENT: re-exec ourselves as the child with a fresh temp root + corpus.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("out");
    let file = tmp.path().join("corpus").join("note.txt");
    std::fs::create_dir_all(file.parent().expect("parent")).expect("create_dir_all");
    write_text(&file, "some words here");

    let exe = std::env::current_exe().expect("current_exe");
    let status = std::process::Command::new(exe)
        .args([
            "stat_index_flush_guard_persists_on_process_exit",
            "--exact",
            "--nocapture",
        ])
        .env(ROOT_ENV, &root)
        .env(FILE_ENV, &file)
        .env_remove("GRAPHIFY_OUT")
        .status()
        .expect("spawn child test process");
    assert!(status.success(), "child test process must exit 0");

    // The child never called flush_stat_index(); only the guard's drop did.
    let idx = root
        .join("graphify-out")
        .join("cache")
        .join("stat-index.json");
    let text = std::fs::read_to_string(&idx)
        .expect("the stat index must be flushed to disk on the child's normal exit");
    assert!(
        text.contains("note.txt"),
        "the flushed index must hold the counted file: {text}"
    );
}

// ── #1259: frontmatter delimiters must be whole `---` lines ──────────────────

#[test]
fn body_content_hr_start_is_not_frontmatter() {
    // A document opening with a `----` thematic break has no frontmatter.
    let content = b"----\nIntro paragraph that must be hashed.\n\n---\nbody";
    assert_eq!(body_content(content), content);
}

#[test]
fn body_content_dash_title_start_is_not_frontmatter() {
    // `--- title` on the first line is prose, not an open delimiter.
    let content = b"--- title\nIntro that must be hashed.\n\n---\nbody";
    assert_eq!(body_content(content), content);
}

#[test]
fn body_content_dash_text_line_is_not_close_delimiter() {
    // `--- text` and `----` lines inside opened frontmatter are not closers.
    let content = b"---\ntitle: Test\nbody starts here\n--- not a delimiter\n----\nreal content";
    assert_eq!(body_content(content), content);
}

#[test]
fn body_content_later_proper_close_skips_dash_text_lines() {
    // A `--- text` line is skipped; the next whole `---` line closes.
    let content = b"---\ntitle: Test\nnote: --- inline\n---\nreal body";
    assert_eq!(body_content(content), b"\nreal body");
}

#[test]
fn body_content_well_formed_output_byte_identical() {
    // For well-formed frontmatter the stripped body stays byte-identical with
    // the historical `text.find("\n---")+4` slice so cache hashes do not churn.
    let cases: &[(&[u8], &[u8])] = &[
        (
            b"---\ntitle: Test\n---\n\nActual body.",
            b"\n\nActual body.",
        ),
        (
            b"---\nreviewed: 2026-01-01\n---\n\n# Title\n\nBody text.",
            b"\n\n# Title\n\nBody text.",
        ),
        // trailing whitespace on the closing delimiter line
        (b"---\ntitle: Test\n---  \nbody", b"  \nbody"),
        // CRLF line endings
        (b"---\r\ntitle: Test\r\n---\r\nbody", b"\r\nbody"),
        // empty frontmatter block
        (b"---\n---\nbody", b"\nbody"),
        // frontmatter with no body
        (b"---\ntitle: Test\n---", b""),
    ];
    for &(content, expected) in cases {
        assert_eq!(
            body_content(content).as_slice(),
            expected,
            "content: {content:?}"
        );
    }
}

#[test]
#[serial]
fn md_edit_above_hr_changes_hash() {
    // Editing content above a mid-document `----` break changes the hash.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "----\nIntro paragraph.\n\n---\nbody");
    let h1 = file_hash(&f, tmp.path(), None).expect("hash");
    bump_mtime(&f);
    write_text(&f, "----\nEdited intro paragraph.\n\n---\nbody");
    let h2 = file_hash(&f, tmp.path(), None).expect("hash");
    assert_ne!(h1, h2);
}

// ── #777: portable cache source_file fields ─────────────────────────────────

#[test]
#[serial]
fn save_cached_relativizes_source_file() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).expect("mkdir");
    let src = root.join("src").join("foo.py");
    write_text(&src, "def x(): pass\n");
    let abs_src = src
        .canonicalize()
        .expect("canon")
        .to_string_lossy()
        .into_owned();
    let result = json!({
        "nodes": [{"id": "n1", "label": "foo", "source_file": abs_src.clone()}],
        "edges": [{"source": "n1", "target": "n1", "source_file": abs_src}],
    });
    save_cached(&src, &result, root, "ast", None).expect("save");

    let h = file_hash(&src, root, None).expect("hash");
    let entry = cache_dir(root, "ast")
        .expect("dir")
        .join(format!("{h}.json"));
    let on_disk: Value =
        serde_json::from_str(&fs::read_to_string(&entry).expect("read")).expect("parse");
    assert_eq!(on_disk["nodes"][0]["source_file"], json!("src/foo.py"));
    assert_eq!(on_disk["edges"][0]["source_file"], json!("src/foo.py"));
}

#[test]
#[serial]
fn load_cached_absolutizes_source_file() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).expect("mkdir");
    let src = root.join("src").join("foo.py");
    write_text(&src, "def x(): pass\n");
    let abs_src = src
        .canonicalize()
        .expect("canon")
        .to_string_lossy()
        .into_owned();
    save_cached(
        &src,
        &json!({
            "nodes": [{"id": "n1", "source_file": abs_src.clone()}],
            "edges": [{"source": "n1", "target": "n1", "source_file": abs_src.clone()}],
        }),
        root,
        "ast",
        None,
    )
    .expect("save");

    let loaded = load_cached(&src, root, "ast", None).expect("loaded");
    assert_eq!(loaded["nodes"][0]["source_file"], json!(abs_src));
    assert_eq!(loaded["edges"][0]["source_file"], json!(abs_src));
}

#[test]
#[serial]
fn load_cached_passes_through_legacy_absolute_source_file() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).expect("mkdir");
    let src = root.join("src").join("foo.py");
    write_text(&src, "pass\n");
    let abs_src = src
        .canonicalize()
        .expect("canon")
        .to_string_lossy()
        .into_owned();

    // Hand-write a legacy-format cache entry (absolute source_file).
    let h = file_hash(&src, root, None).expect("hash");
    let entry = cache_dir(root, "ast")
        .expect("dir")
        .join(format!("{h}.json"));
    let payload = serde_json::to_string(
        &json!({"nodes": [{"id": "n1", "source_file": abs_src.clone()}], "edges": []}),
    )
    .expect("ser");
    fs::write(&entry, payload).expect("write");

    let loaded = load_cached(&src, root, "ast", None).expect("loaded");
    assert_eq!(loaded["nodes"][0]["source_file"], json!(abs_src));
}

#[test]
#[serial]
fn cache_portable_across_roots() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo_a = tmp.path().join("repo_a");
    fs::create_dir_all(repo_a.join("src")).expect("mkdir a");
    let src_a = repo_a.join("src").join("foo.py");
    write_text(&src_a, "def x(): pass\n");
    let abs_a = src_a
        .canonicalize()
        .expect("canon")
        .to_string_lossy()
        .into_owned();
    save_cached(
        &src_a,
        &json!({"nodes": [{"id": "n1", "source_file": abs_a}], "edges": []}),
        &repo_a,
        "ast",
        None,
    )
    .expect("save");

    // Copy corpus + cache to a second location with a different prefix.
    let repo_b = tmp.path().join("repo_b");
    copy_dir_all(&repo_a, &repo_b);

    let src_b = repo_b.join("src").join("foo.py");
    let loaded = load_cached(&src_b, &repo_b, "ast", None).expect("portable");
    let abs_b = src_b
        .canonicalize()
        .expect("canon b")
        .to_string_lossy()
        .into_owned();
    assert_eq!(loaded["nodes"][0]["source_file"], json!(abs_b));
    let sf = loaded["nodes"][0]["source_file"]
        .as_str()
        .expect("string source_file");
    assert!(sf.contains("repo_b"));
    assert!(!sf.contains("repo_a"));
}

// ── AST cache versioning ────────────────────────────────────────────────────

#[test]
#[serial]
fn ast_cache_invalidated_on_version_bump() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let f = root.join("mod.py");
    write_text(&f, "def f(): pass\n");

    save_cached_versioned(
        &f,
        &json!({"nodes": [{"id": "n1"}], "edges": []}),
        root,
        "ast",
        "0.8.0",
        None,
    )
    .expect("save");
    assert!(load_cached_versioned(&f, root, "ast", "0.8.0", None).is_some());
    assert!(
        load_cached_versioned(&f, root, "ast", "0.8.1", None).is_none(),
        "AST cache entry from a previous version must not be served"
    );
}

#[test]
#[serial]
fn ast_cache_version_bump_cleans_stale_entries() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let f = root.join("mod.py");
    write_text(&f, "def f(): pass\n");

    save_cached_versioned(
        &f,
        &json!({"nodes": [{"id": "n1"}], "edges": []}),
        root,
        "ast",
        "0.8.0",
        None,
    )
    .expect("save");
    let old_dir = cache_dir_versioned(root, "ast", "0.8.0").expect("old dir");
    assert!(dir_has_json(&old_dir), "v0.8.0 entry should exist");

    cache_dir_versioned(root, "ast", "0.8.1").expect("bump triggers cleanup");
    assert!(
        !old_dir.exists(),
        "stale AST version directory must be removed on upgrade"
    );
}

#[test]
#[serial]
fn legacy_unversioned_ast_entries_not_served() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let f = root.join("mod.py");
    write_text(&f, "def f(): pass\n");
    let h = file_hash(&f, root, None).expect("hash");
    let payload =
        serde_json::to_string(&json!({"nodes": [{"id": "stale"}], "edges": []})).expect("ser");

    // Derive the real (canonicalised) cache directories from the library so the
    // hand-written stale entries land where load_cached actually looks.
    let versioned = cache_dir(root, "ast").expect("dir");
    let ast_base = versioned.parent().expect("ast base").to_path_buf();
    let cache_base = ast_base.parent().expect("cache base").to_path_buf();
    // Unversioned cache/ast/{hash}.json (pre-versioning layout)
    fs::write(ast_base.join(format!("{h}.json")), &payload).expect("write unversioned");
    // Legacy flat cache/{hash}.json (pre-0.5.3 layout)
    fs::write(cache_base.join(format!("{h}.json")), &payload).expect("write legacy");

    assert!(load_cached(&f, root, "ast", None).is_none());
}

#[test]
#[serial]
fn semantic_cache_survives_version_bump() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let f = root.join("doc.md");
    write_text(&f, "# Title\n\nBody.\n");

    save_cached_versioned(
        &f,
        &json!({"nodes": [{"id": "n1"}], "edges": []}),
        root,
        "semantic",
        "0.8.0",
        None,
    )
    .expect("save");
    let semantic_dir = cache_dir(root, "semantic").expect("semantic dir");

    // A version bump triggers AST cleanup; the semantic cache must survive.
    cache_dir_versioned(root, "ast", "0.8.1").expect("bump");
    assert!(load_cached_versioned(&f, root, "semantic", "0.8.1", None).is_some());
    assert!(
        dir_has_json(&semantic_dir),
        "semantic entries must survive the version bump and AST cleanup"
    );
}

#[cfg(unix)]
#[test]
#[serial]
fn save_cached_in_root_symlink_keeps_symlink_name() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("sub")).expect("mkdir");
    let target = root.join("sub").join("target.py");
    write_text(&target, "pass\n");
    let alias = root.join("alias.py");
    std::os::unix::fs::symlink(&target, &alias).expect("symlink");

    // The caller's view is the symlink path under the resolved root, NOT the
    // resolved target — relativization must keep the symlink's own name.
    let abs_alias = root
        .canonicalize()
        .expect("canon root")
        .join("alias.py")
        .to_string_lossy()
        .into_owned();
    save_cached(
        &alias,
        &json!({"nodes": [{"id": "n1", "source_file": abs_alias}], "edges": []}),
        root,
        "ast",
        None,
    )
    .expect("save");

    let h = file_hash(&alias, root, None).expect("hash");
    let entry = cache_dir(root, "ast")
        .expect("dir")
        .join(format!("{h}.json"));
    let on_disk: Value =
        serde_json::from_str(&fs::read_to_string(&entry).expect("read")).expect("parse");
    assert_eq!(on_disk["nodes"][0]["source_file"], json!("alias.py"));
}

/// Recursively copy `src` into `dst` (test helper; std has no equivalent).
fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("mkdir dst");
    for entry in fs::read_dir(src).expect("readdir") {
        let entry = entry.expect("entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type().expect("ftype").is_dir() {
            copy_dir_all(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy");
        }
    }
}

/// True if `dir` contains at least one `*.json` file (non-recursive).
fn dir_has_json(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten()
            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
    })
}

#[test]
#[serial]
fn semantic_prune_removes_orphan_entries() {
    // Changing a file's content leaves the old content-hash entry orphaned;
    // pruning against the new live hash removes the stale entry, keeps the current.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# A\n\nContent A.\n");
    let h_a = file_hash(&f, tmp.path(), None).expect("h_a");
    save_cached(
        &f,
        &json!({"nodes": [{"id": "a"}], "edges": []}),
        tmp.path(),
        "semantic",
        None,
    )
    .expect("save a");

    write_text(&f, "# B\n\nContent B.\n");
    _reset_stat_index_for_tests();
    bump_mtime(&f);
    let h_b = file_hash(&f, tmp.path(), None).expect("h_b");
    save_cached(
        &f,
        &json!({"nodes": [{"id": "b"}], "edges": []}),
        tmp.path(),
        "semantic",
        None,
    )
    .expect("save b");
    assert_ne!(h_a, h_b, "content change must produce a new hash");

    let semantic_dir = cache_dir(tmp.path(), "semantic").expect("cache_dir");
    assert!(semantic_dir.join(format!("{h_a}.json")).exists());
    assert!(semantic_dir.join(format!("{h_b}.json")).exists());

    let pruned = prune_semantic_cache(tmp.path(), &HashSet::from([h_b.clone()]));
    assert_eq!(pruned, 1);
    assert!(!semantic_dir.join(format!("{h_a}.json")).exists());
    assert!(semantic_dir.join(format!("{h_b}.json")).exists());
}

#[test]
#[serial]
fn semantic_prune_keeps_live_unchanged_entries() {
    // Pruning against the FULL live set must keep every live entry — guards the
    // trap of pruning against an incremental changed-subset.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut live_hashes: HashSet<String> = HashSet::new();
    for i in 0..5 {
        let f = tmp.path().join(format!("doc{i}.md"));
        write_text(&f, &format!("# Doc {i}\n\nBody {i}.\n"));
        save_cached(
            &f,
            &json!({"nodes": [{"id": i.to_string()}], "edges": []}),
            tmp.path(),
            "semantic",
            None,
        )
        .expect("save");
        live_hashes.insert(file_hash(&f, tmp.path(), None).expect("hash"));
    }
    let semantic_dir = cache_dir(tmp.path(), "semantic").expect("cache_dir");
    let count = || {
        fs::read_dir(&semantic_dir)
            .expect("read_dir")
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count()
    };
    assert_eq!(count(), 5);
    assert_eq!(prune_semantic_cache(tmp.path(), &live_hashes), 0);
    assert_eq!(count(), 5);
}

#[test]
#[serial]
fn semantic_prune_handles_deleted_file() {
    // An entry for a file that no longer exists (dropped from the live set) is pruned.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("gone.md");
    write_text(&f, "# Gone\n\nWill be deleted.\n");
    let h = file_hash(&f, tmp.path(), None).expect("hash");
    save_cached(
        &f,
        &json!({"nodes": [{"id": "g"}], "edges": []}),
        tmp.path(),
        "semantic",
        None,
    )
    .expect("save");
    let semantic_dir = cache_dir(tmp.path(), "semantic").expect("cache_dir");
    assert!(semantic_dir.join(format!("{h}.json")).exists());

    fs::remove_file(&f).expect("unlink");
    let pruned = prune_semantic_cache(tmp.path(), &HashSet::new());
    assert_eq!(pruned, 1);
    assert!(!semantic_dir.join(format!("{h}.json")).exists());
}

#[test]
#[serial]
fn semantic_prune_ignores_ast_and_tmp() {
    // Prune touches only cache/semantic/*.json: AST entries and atomic-write
    // *.tmp temporaries are left untouched.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Doc\n\nBody.\n");
    // AST entry (different subtree) must survive.
    save_cached(
        &f,
        &json!({"nodes": [{"id": "ast"}], "edges": []}),
        tmp.path(),
        "ast",
        None,
    )
    .expect("save ast");
    let ast_dir = cache_dir(tmp.path(), "ast").expect("ast dir");
    let ast_json = |d: &Path| {
        fs::read_dir(d)
            .expect("read_dir")
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count()
    };
    assert_eq!(ast_json(&ast_dir), 1);

    // A semantic orphan .json (to be pruned) plus a .tmp temporary (to survive).
    let semantic_dir = cache_dir(tmp.path(), "semantic").expect("semantic dir");
    write_text(
        &semantic_dir.join("deadbeef.json"),
        "{\"nodes\": [], \"edges\": []}",
    );
    let tmp_entry = semantic_dir.join("deadbeef.tmp");
    write_text(&tmp_entry, "partial");

    let pruned = prune_semantic_cache(tmp.path(), &HashSet::new());
    assert_eq!(pruned, 1);
    assert!(!semantic_dir.join("deadbeef.json").exists());
    assert!(tmp_entry.exists(), "*.tmp temporaries must not be swept");
    assert_eq!(ast_json(&ast_dir), 1, "AST entries must not be touched");
}

#[test]
#[serial]
fn test_save_semantic_cache_overwrites_by_default() {
    // Default save_semantic_cache replaces a file's cached entry (the final,
    // authoritative write in the extract pipeline).
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Doc\n");
    save_semantic_cache(
        &[json!({"id": "a", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        graphify_cache::SemanticCacheOptions {
            merge_existing: false,
            allowed_source_files: None,
            ..Default::default()
        },
    )
    .expect("save 1");
    save_semantic_cache(
        &[json!({"id": "b", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        graphify_cache::SemanticCacheOptions {
            merge_existing: false,
            allowed_source_files: None,
            ..Default::default()
        },
    )
    .expect("save 2");
    let cached = load_cached(&f, tmp.path(), "semantic", None).expect("cached");
    let ids: HashSet<&str> = cached["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        HashSet::from(["b"]),
        "default must overwrite, not accumulate"
    );
}

#[test]
#[serial]
fn test_save_semantic_cache_rejects_out_of_scope_source_file() {
    // #1757: an undispatched file must keep its complete cache entry when a
    // semantic result misattributes a node to it.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let intended = root.join("intended.md");
    write_text(&intended, "# Intended\n");
    let protected = root.join("protected.md");
    write_text(&protected, "# Protected\n");

    // Seed the protected file's cache entry.
    save_semantic_cache(
        &[json!({"id": "original", "source_file": "protected.md"})],
        &[],
        &[],
        &root,
        graphify_cache::SemanticCacheOptions {
            merge_existing: false,
            allowed_source_files: None,
            ..Default::default()
        },
    )
    .expect("seed");

    let nodes = [
        json!({"id": "expected", "source_file": intended.to_string_lossy()}),
        json!({"id": "stray", "source_file": "protected.md"}),
    ];
    let edges = [json!({"source": "stray", "target": "expected", "source_file": "protected.md"})];
    let hyperedges =
        [json!({"id": "stray_hyperedge", "nodes": ["stray"], "source_file": "protected.md"})];
    let allowed = [std::path::PathBuf::from("intended.md")];
    let saved = save_semantic_cache(
        &nodes,
        &edges,
        &hyperedges,
        &root,
        graphify_cache::SemanticCacheOptions {
            merge_existing: false,
            allowed_source_files: Some(&allowed),
            ..Default::default()
        },
    )
    .expect("save");
    assert_eq!(saved, 1, "only the dispatched file may be written");

    let intended_cache = load_cached(&intended, &root, "semantic", None).expect("intended cache");
    let ids: HashSet<&str> = intended_cache["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert_eq!(ids, HashSet::from(["expected"]));
    // The stray edge/hyperedge carry `source_file: protected.md` (out of the
    // allowlist), so they must not leak into intended.md's cache either.
    let edge_ids: HashSet<&str> = intended_cache["edges"]
        .as_array()
        .expect("edges")
        .iter()
        .filter_map(|e| e["source"].as_str())
        .collect();
    assert!(
        !edge_ids.contains("stray"),
        "stray edge leaked into intended cache"
    );
    let hyper_ids: HashSet<&str> = intended_cache["hyperedges"]
        .as_array()
        .expect("hyperedges")
        .iter()
        .filter_map(|h| h["id"].as_str())
        .collect();
    assert!(
        !hyper_ids.contains("stray_hyperedge"),
        "stray hyperedge leaked into intended cache"
    );

    // The protected file keeps its original entry, untouched by the stray node.
    let protected_cache =
        load_cached(&protected, &root, "semantic", None).expect("protected cache");
    let pids: HashSet<&str> = protected_cache["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert_eq!(pids, HashSet::from(["original"]));
    assert!(
        protected_cache["edges"]
            .as_array()
            .expect("edges")
            .is_empty()
    );
    assert!(
        protected_cache["hyperedges"]
            .as_array()
            .expect("hyperedges")
            .is_empty()
    );
}

#[test]
#[serial]
fn test_save_semantic_cache_merge_existing_unions() {
    // #1715: merge_existing=true concatenates (prev + new, ordered, no dedup)
    // across all three arrays so a file split across chunks keeps every slice.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("big.md");
    write_text(&f, "# Big\n");
    save_semantic_cache(
        &[json!({"id": "a", "source_file": "big.md"})],
        &[json!({"source": "a", "target": "x", "source_file": "big.md"})],
        &[json!({"id": "h1", "nodes": ["a"], "source_file": "big.md"})],
        tmp.path(),
        graphify_cache::SemanticCacheOptions {
            merge_existing: true,
            allowed_source_files: None,
            ..Default::default()
        },
    )
    .expect("chunk 1");
    save_semantic_cache(
        &[json!({"id": "b", "source_file": "big.md"})],
        &[json!({"source": "b", "target": "y", "source_file": "big.md"})],
        &[json!({"id": "h2", "nodes": ["b"], "source_file": "big.md"})],
        tmp.path(),
        graphify_cache::SemanticCacheOptions {
            merge_existing: true,
            allowed_source_files: None,
            ..Default::default()
        },
    )
    .expect("chunk 2");
    let cached = load_cached(&f, tmp.path(), "semantic", None).expect("cached");
    let field = |k: &str, id_key: &str| -> Vec<String> {
        cached[k]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|v| v[id_key].as_str().map(str::to_string))
            .collect()
    };
    // Ordered prev + new, no dedup — verifies both accumulation and ordering.
    assert_eq!(field("nodes", "id"), vec!["a", "b"]);
    assert_eq!(field("edges", "source"), vec!["a", "b"]);
    assert_eq!(field("hyperedges", "id"), vec!["h1", "h2"]);
}

// ── #1656: word-count caching ─────────────────────────────────────────────────
// Ports `graphify-py/tests/test_word_count_cache.py`. Word counts are cached
// against each file's stat signature so `detect()` doesn't re-parse every
// unchanged PDF/docx on each run just to size the corpus.

#[test]
#[serial]
fn word_count_cached_until_file_changes() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.txt");
    write_text(&f, "one two three four five");

    let calls = std::cell::Cell::new(0u32);
    let compute = |p: &Path| -> u64 {
        calls.set(calls.get() + 1);
        fs::read_to_string(p)
            .unwrap_or_default()
            .split_whitespace()
            .count() as u64
    };

    assert_eq!(cached_word_count(&f, tmp.path(), compute, None), 5);
    assert_eq!(calls.get(), 1);
    // Second call, file unchanged → served from cache, compute NOT re-run.
    assert_eq!(cached_word_count(&f, tmp.path(), compute, None), 5);
    assert_eq!(calls.get(), 1);

    // Change the file → recompute.
    write_text(&f, "only three words now");
    bump_mtime(&f);
    assert_eq!(cached_word_count(&f, tmp.path(), compute, None), 4);
    assert_eq!(calls.get(), 2);
}

#[test]
#[serial]
fn word_count_augments_existing_hash_entry() {
    // `cached_word_count` must not clobber a hash already stored for the file: the
    // hash still resolves from the fastpath afterwards, and the count is correct.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("m.py");
    write_text(&f, "x = 1\n"); // -> ["x", "=", "1"] == 3 tokens
    let h = file_hash(&f, tmp.path(), None).expect("hash");
    assert!(!h.is_empty());
    let wc = cached_word_count(
        &f,
        tmp.path(),
        |p| {
            fs::read_to_string(p)
                .unwrap_or_default()
                .split_whitespace()
                .count() as u64
        },
        None,
    );
    assert_eq!(wc, 3);
    // The hash entry survives alongside the word_count (fastpath still returns it).
    assert_eq!(file_hash(&f, tmp.path(), None).expect("hash"), h);
}

/// #1747 / root-keyed stat index: two `cached_word_count` invocations with
/// DIFFERENT `cache_root`s must each persist to their OWN cache-file root, not
/// share the first-seen one. Before the fix the process-global index ignored
/// every root after the first, so the second file's entry was written into the
/// first root's index.
#[test]
#[serial]
fn cache_root_is_per_invocation_not_first_seen() {
    _reset_stat_index_for_tests();
    let corpus = tempfile::tempdir().expect("tempdir");
    let root_a = tempfile::tempdir().expect("tempdir");
    let root_b = tempfile::tempdir().expect("tempdir");
    let fa = corpus.path().join("a.txt");
    let fb = corpus.path().join("b.txt");
    write_text(&fa, "alpha");
    write_text(&fb, "beta");

    // Two runs, two distinct cache roots.
    assert_eq!(
        cached_word_count(&fa, corpus.path(), |_| 3, Some(root_a.path())),
        3
    );
    assert_eq!(
        cached_word_count(&fb, corpus.path(), |_| 5, Some(root_b.path())),
        5
    );
    flush_stat_index().expect("flush");

    let idx_a = root_a
        .path()
        .join("graphify-out")
        .join("cache")
        .join("stat-index.json");
    let idx_b = root_b
        .path()
        .join("graphify-out")
        .join("cache")
        .join("stat-index.json");
    let text_a = std::fs::read_to_string(&idx_a).expect("root A index must exist");
    let text_b = std::fs::read_to_string(&idx_b).expect("root B index must exist");

    // Each root's index holds ONLY its own file — no cross-contamination.
    assert!(text_a.contains("a.txt"), "root A must cache a.txt");
    assert!(!text_a.contains("b.txt"), "root A must not hold b.txt");
    assert!(text_b.contains("b.txt"), "root B must cache b.txt");
    assert!(!text_b.contains("a.txt"), "root B must not hold a.txt");
}

/// Restores a `GRAPHIFY_OUT` override on drop so a panic mid-test cannot leak
/// it into other serial tests sharing the process environment.
struct GraphifyOutGuard {
    prev: Option<String>,
}

impl GraphifyOutGuard {
    fn set(value: &Path) -> Self {
        let prev = std::env::var("GRAPHIFY_OUT").ok();
        unsafe { std::env::set_var("GRAPHIFY_OUT", value) };
        Self { prev }
    }
}

impl Drop for GraphifyOutGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => unsafe { std::env::set_var("GRAPHIFY_OUT", v) },
            None => unsafe { std::env::remove_var("GRAPHIFY_OUT") },
        }
    }
}

/// #1747 / root-keyed stat index: with an ABSOLUTE `GRAPHIFY_OUT`, `out_base`
/// ignores the cache root, so every root resolves to the SAME stat-index file.
/// The two runs must then SHARE one state and merge into that single file, not
/// compete and clobber each other.
#[test]
#[serial]
fn absolute_graphify_out_shares_one_index_across_roots() {
    _reset_stat_index_for_tests();
    let out = tempfile::tempdir().expect("tempdir");
    let out_abs = out.path().join("shared-out");
    let _out_guard = GraphifyOutGuard::set(&out_abs);

    let corpus = tempfile::tempdir().expect("tempdir");
    let root_a = tempfile::tempdir().expect("tempdir");
    let root_b = tempfile::tempdir().expect("tempdir");
    let fa = corpus.path().join("a.txt");
    let fb = corpus.path().join("b.txt");
    write_text(&fa, "alpha");
    write_text(&fb, "beta");

    assert_eq!(
        cached_word_count(&fa, corpus.path(), |_| 3, Some(root_a.path())),
        3
    );
    assert_eq!(
        cached_word_count(&fb, corpus.path(), |_| 5, Some(root_b.path())),
        5
    );
    flush_stat_index().expect("flush");

    let idx = out_abs.join("cache").join("stat-index.json");
    let text = std::fs::read_to_string(&idx).expect("the shared index file must exist");
    assert!(text.contains("a.txt"), "shared index must hold a.txt");
    assert!(
        text.contains("b.txt"),
        "shared index must hold b.txt too (merged, not clobbered)"
    );
    _reset_stat_index_for_tests();
}

/// When the cache dir does not exist yet (`canonicalize` fails), two spellings
/// of the same absolute dir must still share ONE stat index: the key is a
/// normalized absolute path, not the raw string. A relative/raw fallback would
/// split `<d>/out` and `<d>/out/.` into competing indexes.
#[test]
#[serial]
fn nonexistent_cache_root_keys_by_normalized_absolute_path() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tempfile::tempdir().expect("tempdir");
    let fa = corpus.path().join("a.txt");
    let fb = corpus.path().join("b.txt");
    write_text(&fa, "alpha");
    write_text(&fb, "beta");
    // Neither exists yet — canonicalize fails, exercising the absolute fallback.
    let root_plain = tmp.path().join("out");
    let root_dotted = tmp.path().join("out").join(".");

    assert_eq!(
        cached_word_count(&fa, corpus.path(), |_| 3, Some(&root_plain)),
        3
    );
    assert_eq!(
        cached_word_count(&fb, corpus.path(), |_| 5, Some(&root_dotted)),
        5
    );
    flush_stat_index().expect("flush");

    let idx = tmp
        .path()
        .join("out")
        .join("graphify-out")
        .join("cache")
        .join("stat-index.json");
    let text = std::fs::read_to_string(&idx).expect("the single normalized index must exist");
    assert!(
        text.contains("a.txt") && text.contains("b.txt"),
        "both spellings must share one index: {text}"
    );
    _reset_stat_index_for_tests();
}

/// `flush_stat_index` must persist every dirty root even when one fails: a
/// single unwritable cache dir cannot strand another root's entries.
#[test]
#[serial]
fn flush_continues_past_a_failing_root() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let corpus = tempfile::tempdir().expect("tempdir");
    let fb = corpus.path().join("b.txt");
    let fa = corpus.path().join("a.txt");
    write_text(&fb, "beta");
    write_text(&fa, "alpha");
    // Root B's cache dir cannot be created — an ancestor is a regular file.
    let blocker = tmp.path().join("blocker");
    write_text(&blocker, "not a dir");
    let root_bad = blocker.join("sub");
    let root_good = tmp.path().join("good");

    // Populate the bad root first so it is flushed before the good one.
    cached_word_count(&fb, corpus.path(), |_| 5, Some(&root_bad));
    cached_word_count(&fa, corpus.path(), |_| 3, Some(&root_good));

    assert!(
        flush_stat_index().is_err(),
        "the failing root's error must be surfaced"
    );
    let good_idx = root_good
        .join("graphify-out")
        .join("cache")
        .join("stat-index.json");
    assert!(
        good_idx.exists(),
        "the healthy root must still be flushed despite the failing one"
    );
    _reset_stat_index_for_tests();
}

// ── #1894: mode-namespaced semantic cache + #1916 dangling-ref pruning ────────

#[must_use]
fn ids(nodes: &[Value]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[must_use]
fn deep_opts<'a>() -> SemanticCacheOptions<'a> {
    SemanticCacheOptions {
        mode: Some("deep"),
        ..Default::default()
    }
}

#[test]
#[serial]
fn semantic_cache_deep_mode_roundtrip_under_deep_namespace() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Doc\n\nBody.\n");
    let saved = save_semantic_cache(
        &[json!({"id": "deep_n", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        deep_opts(),
    )
    .expect("save");
    assert_eq!(saved, 1);

    let h = file_hash(&f, tmp.path(), None).expect("hash");
    let base = tmp.path().join("graphify-out").join("cache");
    assert!(
        base.join("semantic-deep")
            .join(format!("{h}.json"))
            .exists()
    );
    assert!(!base.join("semantic").join(format!("{h}.json")).exists());

    let split = check_semantic_cache(
        &[f.to_string_lossy().into_owned()],
        tmp.path(),
        Some("deep"),
    );
    assert_eq!(ids(&split.cached_nodes), ["deep_n"]);
    assert!(split.uncached_files.is_empty());
}

#[test]
#[serial]
fn remove_semantic_cache_entries_evicts_only_named_files() {
    // A forced re-extraction that returns no records for a dispatched file must
    // evict that file's PRIOR entry so the next run MISSES (re-extracts) instead
    // of serving a stale (e.g. pre-model-change) result — while a sibling entry
    // and other namespaces stay intact.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("a.md");
    write_text(&a, "# A\n\nBody A.\n");
    let b = tmp.path().join("b.md");
    write_text(&b, "# B\n\nBody B.\n");

    // Warm the standard namespace for both, plus a deep entry for `a`.
    save_semantic_cache(
        &[json!({"id": "na", "source_file": "a.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save a");
    save_semantic_cache(
        &[json!({"id": "nb", "source_file": "b.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save b");
    save_semantic_cache(
        &[json!({"id": "da", "source_file": "a.md"})],
        &[],
        &[],
        tmp.path(),
        deep_opts(),
    )
    .expect("save deep a");

    // Evict only `a` from the standard namespace.
    assert_eq!(
        remove_semantic_cache_entries(std::slice::from_ref(&a), tmp.path(), None),
        1
    );
    // A second call is a no-op (entry already gone).
    assert_eq!(
        remove_semantic_cache_entries(std::slice::from_ref(&a), tmp.path(), None),
        0
    );

    // `a` now MISSES in the standard namespace; `b` still hits.
    let split = check_semantic_cache(
        &[
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ],
        tmp.path(),
        None,
    );
    assert_eq!(split.uncached_files, [a.to_string_lossy().into_owned()]);
    assert_eq!(ids(&split.cached_nodes), ["nb"]);

    // The deep-namespace entry for `a` is untouched (mode-scoped eviction).
    let deep = check_semantic_cache(
        &[a.to_string_lossy().into_owned()],
        tmp.path(),
        Some("deep"),
    );
    assert_eq!(ids(&deep.cached_nodes), ["da"]);
}

#[cfg(unix)]
#[test]
#[serial]
fn remove_semantic_cache_entries_never_follows_symlinked_namespace() {
    // #1894 hardening: eviction must not unlink THROUGH a symlinked `semantic/`
    // namespace into an external tree — a JSON in the pointed-at dir survives.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let a = tmp.path().join("a.md");
    write_text(&a, "# A\n\nBody A.\n");
    // The hash an eviction WOULD target if it followed the link.
    let h = file_hash(&a, tmp.path(), None).expect("hash");
    let outside = tempfile::tempdir().expect("outside");
    let external = outside.path().join(format!("{h}.json"));
    write_text(&external, "{}");

    let cache = tmp.path().join("graphify-out").join("cache");
    std::fs::create_dir_all(&cache).expect("mkdir cache");
    std::os::unix::fs::symlink(outside.path(), cache.join("semantic")).expect("symlink");

    // The symlinked namespace is skipped, so nothing is removed and the external
    // target is untouched.
    assert_eq!(
        remove_semantic_cache_entries(std::slice::from_ref(&a), tmp.path(), None),
        0
    );
    assert!(external.exists(), "eviction followed a symlinked namespace");
}

#[test]
#[serial]
fn semantic_cache_deep_invisible_to_plain_reads_and_vice_versa() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let deep_doc = tmp.path().join("deep.md");
    write_text(&deep_doc, "# Deep\n");
    let plain_doc = tmp.path().join("plain.md");
    write_text(&plain_doc, "# Plain\n");
    save_semantic_cache(
        &[json!({"id": "d", "source_file": "deep.md"})],
        &[],
        &[],
        tmp.path(),
        deep_opts(),
    )
    .expect("save deep");
    save_semantic_cache(
        &[json!({"id": "p", "source_file": "plain.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save plain");

    let paths = [
        deep_doc.to_string_lossy().into_owned(),
        plain_doc.to_string_lossy().into_owned(),
    ];
    let plain = check_semantic_cache(&paths, tmp.path(), None);
    assert_eq!(ids(&plain.cached_nodes), ["p"]);
    assert_eq!(
        plain.uncached_files,
        [deep_doc.to_string_lossy().into_owned()]
    );

    let deep = check_semantic_cache(&paths, tmp.path(), Some("deep"));
    assert_eq!(ids(&deep.cached_nodes), ["d"]);
    assert_eq!(
        deep.uncached_files,
        [plain_doc.to_string_lossy().into_owned()]
    );
}

#[test]
#[serial]
fn semantic_cache_mode_none_layout_unchanged() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Doc\n");
    save_semantic_cache(
        &[json!({"id": "n", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save");
    let h = file_hash(&f, tmp.path(), None).expect("hash");
    let base = tmp.path().join("graphify-out").join("cache");
    assert!(base.join("semantic").join(format!("{h}.json")).exists());
    assert!(
        !base.join("semantic-deep").exists(),
        "mode=None must never create the deep namespace"
    );
    let split = check_semantic_cache(&[f.to_string_lossy().into_owned()], tmp.path(), None);
    assert_eq!(ids(&split.cached_nodes), ["n"]);
    assert!(split.uncached_files.is_empty());
}

#[test]
#[serial]
fn clear_cache_removes_deep_namespace() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Doc\n");
    save_semantic_cache(
        &[json!({"id": "p", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save p");
    save_semantic_cache(
        &[json!({"id": "d", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        deep_opts(),
    )
    .expect("save d");
    clear_cache(tmp.path()).expect("clear");
    let base = tmp.path().join("graphify-out").join("cache");
    for kind in ["semantic", "semantic-deep"] {
        let dir = base.join(kind);
        let remaining = fs::read_dir(&dir).map_or(0, |rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        });
        assert_eq!(remaining, 0, "clear_cache must sweep {kind}");
    }
}

#[test]
#[serial]
fn cached_files_includes_deep_namespace() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Doc\n");
    save_semantic_cache(
        &[json!({"id": "d", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        deep_opts(),
    )
    .expect("save");
    let h = file_hash(&f, tmp.path(), None).expect("hash");
    assert!(cached_files(tmp.path()).contains(&h));
}

#[test]
#[serial]
fn semantic_namespaces_cover_arbitrary_mode() {
    // #1894: a future `--mode custom` writes `cache/semantic-custom/`. Namespace
    // enumeration must list, prune, and clear it without a hard-coded name — the
    // old code only knew `semantic` + `semantic-deep`.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# Doc\n");
    save_semantic_cache(
        &[json!({"id": "c", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions {
            mode: Some("custom"),
            ..Default::default()
        },
    )
    .expect("save custom");
    let base = tmp.path().join("graphify-out").join("cache");
    let custom_dir = base.join("semantic-custom");
    let h = file_hash(&f, tmp.path(), None).expect("hash");
    assert!(
        custom_dir.join(format!("{h}.json")).exists(),
        "custom-mode entry written to semantic-custom/"
    );
    // cached_files enumerates the custom namespace.
    assert!(
        cached_files(tmp.path()).contains(&h),
        "cached_files must include the custom namespace"
    );
    // prune_semantic_cache sweeps it against an empty live set.
    let empty: HashSet<String> = HashSet::new();
    assert_eq!(
        prune_semantic_cache(tmp.path(), &empty),
        1,
        "custom-namespace orphan must be pruned"
    );
    assert!(!custom_dir.join(format!("{h}.json")).exists());
    // clear_cache sweeps a freshly-saved custom entry too.
    save_semantic_cache(
        &[json!({"id": "c", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions {
            mode: Some("custom"),
            ..Default::default()
        },
    )
    .expect("re-save custom");
    clear_cache(tmp.path()).expect("clear");
    let remaining = fs::read_dir(&custom_dir).map_or(0, |rd| {
        rd.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .count()
    });
    assert_eq!(remaining, 0, "clear_cache must sweep semantic-custom");
}

#[cfg(unix)]
#[test]
#[serial]
fn prune_and_clear_never_follow_symlinked_namespace() {
    // A symlinked `semantic-*` under `cache/` must never be followed: prune must
    // not read_dir it (and delete JSON in the external target), and clear must
    // reject rather than traverse it. A JSON in the pointed-at tree survives.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside");
    let external = outside.path().join("external.json");
    write_text(&external, "{}");
    let cache = tmp.path().join("graphify-out").join("cache");
    std::fs::create_dir_all(&cache).expect("mkdir cache");
    std::os::unix::fs::symlink(outside.path(), cache.join("semantic-evil")).expect("symlink");

    let empty: HashSet<String> = HashSet::new();
    assert_eq!(
        prune_semantic_cache(tmp.path(), &empty),
        0,
        "prune must not follow a symlinked namespace"
    );
    // clear must reject (Err) the symlinked dir under cache/ rather than
    // traversing it — it hits the symlink entry before deleting anything real.
    assert!(
        clear_cache(tmp.path()).is_err(),
        "clear must reject a symlinked namespace, not traverse it"
    );
    assert!(
        external.exists(),
        "JSON in the symlink target must survive prune/clear"
    );
}

#[test]
#[serial]
fn semantic_prune_sweeps_both_namespaces_against_same_live_set() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("doc.md");
    write_text(&f, "# A\n\nContent A.\n");
    let h_old = file_hash(&f, tmp.path(), None).expect("h_old");
    save_semantic_cache(
        &[json!({"id": "pa", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save");
    save_semantic_cache(
        &[json!({"id": "da", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        deep_opts(),
    )
    .expect("save");

    _reset_stat_index_for_tests();
    write_text(&f, "# B\n\nContent B.\n");
    bump_mtime(&f);
    let h_live = file_hash(&f, tmp.path(), None).expect("h_live");
    save_semantic_cache(
        &[json!({"id": "pb", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save");
    save_semantic_cache(
        &[json!({"id": "db", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        deep_opts(),
    )
    .expect("save");

    let base = tmp.path().join("graphify-out").join("cache");
    let plain_dir = base.join("semantic");
    let deep_dir = base.join("semantic-deep");
    for d in [&plain_dir, &deep_dir] {
        assert!(d.join(format!("{h_old}.json")).exists());
        assert!(d.join(format!("{h_live}.json")).exists());
    }

    let live: HashSet<String> = [h_live.clone()].into_iter().collect();
    let pruned = prune_semantic_cache(tmp.path(), &live);
    assert_eq!(pruned, 2, "one orphan in EACH namespace must be pruned");
    for d in [&plain_dir, &deep_dir] {
        assert!(!d.join(format!("{h_old}.json")).exists(), "orphan survived");
        assert!(
            d.join(format!("{h_live}.json")).exists(),
            "live entry pruned"
        );
    }
}
/// Scoped-save option with a single allowed file.
fn scoped(allowed: &[std::path::PathBuf]) -> SemanticCacheOptions<'_> {
    SemanticCacheOptions {
        allowed_source_files: Some(allowed),
        ..Default::default()
    }
}

#[test]
#[serial]
fn save_semantic_cache_drops_edges_to_out_of_scope_nodes() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    write_text(&tmp.path().join("allowed.md"), "# Allowed\n");
    write_text(&tmp.path().join("outside.md"), "# Outside\n");
    let nodes = [
        json!({"id": "kept", "source_file": "allowed.md"}),
        json!({"id": "stray", "source_file": "outside.md"}),
        json!({"id": "dup", "source_file": "allowed.md"}),
        json!({"id": "dup", "source_file": "outside.md"}),
    ];
    let edges = [
        json!({"source": "kept", "target": "stray", "source_file": "allowed.md"}),
        json!({"source": "stray", "target": "kept", "source_file": "allowed.md"}),
        json!({"source": "kept", "target": "dup", "source_file": "allowed.md"}),
    ];
    let allowed = [std::path::PathBuf::from("allowed.md")];
    let saved =
        save_semantic_cache(&nodes, &edges, &[], tmp.path(), scoped(&allowed)).expect("save");
    assert_eq!(saved, 1);

    let split = check_semantic_cache(
        &[tmp.path().join("allowed.md").to_string_lossy().into_owned()],
        tmp.path(),
        None,
    );
    assert!(split.uncached_files.is_empty());
    let node_ids: HashSet<String> = ids(&split.cached_nodes).into_iter().collect();
    assert_eq!(
        node_ids,
        ["kept".to_string(), "dup".to_string()]
            .into_iter()
            .collect()
    );
    let pairs: Vec<(String, String)> = split
        .cached_edges
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                e.get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert_eq!(pairs, [("kept".to_string(), "dup".to_string())]);
}

#[test]
#[serial]
fn save_semantic_cache_drops_edges_to_ghost_file_nodes() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    write_text(&tmp.path().join("real.md"), "# Real\n");
    let nodes = [
        json!({"id": "kept", "source_file": "real.md"}),
        json!({"id": "phantom", "source_file": "ghost.md"}),
    ];
    let edges = [
        json!({"source": "kept", "target": "phantom", "source_file": "real.md"}),
        json!({"source": "kept", "target": "kept", "relation": "self", "source_file": "real.md"}),
    ];
    let allowed = [std::path::PathBuf::from("real.md")];
    let saved =
        save_semantic_cache(&nodes, &edges, &[], tmp.path(), scoped(&allowed)).expect("save");
    assert_eq!(saved, 1);

    let split = check_semantic_cache(
        &[tmp.path().join("real.md").to_string_lossy().into_owned()],
        tmp.path(),
        None,
    );
    assert_eq!(ids(&split.cached_nodes), ["kept"]);
    let pairs: Vec<(String, String)> = split
        .cached_edges
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                e.get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert_eq!(pairs, [("kept".to_string(), "kept".to_string())]);
}

#[test]
#[serial]
fn save_semantic_cache_keeps_edge_to_node_in_retained_entry() {
    // #1916 divergence from graphify-py: an edge whose endpoint the model
    // mis-attributes to a skipped (ghost) group in THIS batch must survive when
    // that id lives in a valid retained entry for another, untouched file — on
    // replay the id is present, so the ref is not dangling. graphify-py prunes it.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    write_text(&tmp.path().join("real.md"), "# Real\n");
    write_text(&tmp.path().join("other.md"), "# Other\n");
    // Seed real.md (untouched by the next batch) with node "shared".
    save_semantic_cache(
        &[json!({"id": "shared", "source_file": "real.md"})],
        &[],
        &[],
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("seed real");
    // Batch: write other.md with an edge to "shared", which the model
    // mis-attributes to a ghost file (a skipped group).
    let nodes = [
        json!({"id": "o1", "source_file": "other.md"}),
        json!({"id": "shared", "source_file": "ghost.md"}),
    ];
    let edges = [json!({"source": "o1", "target": "shared", "source_file": "other.md"})];
    let allowed = [std::path::PathBuf::from("other.md")];
    save_semantic_cache(&nodes, &edges, &[], tmp.path(), scoped(&allowed)).expect("save");

    let split = check_semantic_cache(
        &[tmp.path().join("other.md").to_string_lossy().into_owned()],
        tmp.path(),
        None,
    );
    let pairs: Vec<(String, String)> = split
        .cached_edges
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                e.get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [("o1".to_string(), "shared".to_string())],
        "edge to an id in a retained entry must survive"
    );
}

#[test]
#[serial]
fn dangling_prune_treats_bool_and_numbers_python_equal() {
    // #1916 / scalar_key: Python hashes `True == 1` and `1 == 1.0` as ONE set
    // key, so an edge endpoint `1.0`/`false` that resolves to a ghost (skipped)
    // numeric id must be pruned as dangling — while a STRING "1" stays distinct
    // and survives.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    write_text(&tmp.path().join("real.md"), "# Real\n");
    // Ghost (skipped) group contributes numeric ids 1 and 0.
    let nodes = [
        json!({"id": "anchor", "source_file": "real.md"}),
        json!({"id": 1, "source_file": "ghost.md"}),
        json!({"id": 0, "source_file": "ghost.md"}),
    ];
    let edges = [
        json!({"source": "anchor", "target": 1.0, "source_file": "real.md"}), // == ghost 1 → drop
        json!({"source": "anchor", "target": false, "source_file": "real.md"}), // == ghost 0 → drop
        json!({"source": "anchor", "target": "1", "source_file": "real.md"}), // string ≠ 1 → keep
    ];
    let allowed = [std::path::PathBuf::from("real.md")];
    save_semantic_cache(&nodes, &edges, &[], tmp.path(), scoped(&allowed)).expect("save");

    let split = check_semantic_cache(
        &[tmp.path().join("real.md").to_string_lossy().into_owned()],
        tmp.path(),
        None,
    );
    let targets: Vec<Value> = split
        .cached_edges
        .iter()
        .filter_map(|e| e.get("target").cloned())
        .collect();
    assert_eq!(
        targets,
        [json!("1")],
        "only the string \"1\" edge survives; bool/int/float ghosts are pruned: {targets:?}"
    );
}

#[test]
#[serial]
fn save_semantic_cache_keeps_edge_to_merge_existing_node() {
    // #1916 divergence: under merge_existing the prior slice survives, so an edge
    // to a node from that slice is not dangling even when the current chunk
    // mis-attributes the same id to a ghost group.
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    write_text(&tmp.path().join("big.md"), "# Big\n");
    let allowed = [std::path::PathBuf::from("big.md")];
    let merged = |n: &[Value], e: &[Value]| {
        save_semantic_cache(
            n,
            e,
            &[],
            tmp.path(),
            SemanticCacheOptions {
                merge_existing: true,
                allowed_source_files: Some(&allowed),
                ..Default::default()
            },
        )
        .expect("save");
    };
    // Chunk 1: cache node "shared" for big.md.
    merged(&[json!({"id": "shared", "source_file": "big.md"})], &[]);
    // Chunk 2: new node + edge to "shared", which is now mis-attributed to a ghost.
    merged(
        &[
            json!({"id": "b2", "source_file": "big.md"}),
            json!({"id": "shared", "source_file": "ghost.md"}),
        ],
        &[json!({"source": "b2", "target": "shared", "source_file": "big.md"})],
    );

    let split = check_semantic_cache(
        &[tmp.path().join("big.md").to_string_lossy().into_owned()],
        tmp.path(),
        None,
    );
    let pairs: Vec<(String, String)> = split
        .cached_edges
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                e.get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert!(
        pairs.contains(&("b2".to_string(), "shared".to_string())),
        "edge to a merge_existing prior node must survive: {pairs:?}"
    );
}

#[test]
#[serial]
fn save_semantic_cache_drops_hyperedges_touching_skipped_nodes() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    write_text(&tmp.path().join("allowed.md"), "# Allowed\n");
    write_text(&tmp.path().join("outside.md"), "# Outside\n");
    let nodes = [
        json!({"id": "kept", "source_file": "allowed.md"}),
        json!({"id": "kept2", "source_file": "allowed.md"}),
        json!({"id": "stray", "source_file": "outside.md"}),
    ];
    let hyperedges = [
        json!({"id": "he_bad", "nodes": ["kept", "stray"], "source_file": "allowed.md"}),
        json!({"id": "he_ok", "nodes": ["kept", "kept2"], "source_file": "allowed.md"}),
    ];
    let allowed = [std::path::PathBuf::from("allowed.md")];
    save_semantic_cache(&nodes, &[], &hyperedges, tmp.path(), scoped(&allowed)).expect("save");

    let split = check_semantic_cache(
        &[tmp.path().join("allowed.md").to_string_lossy().into_owned()],
        tmp.path(),
        None,
    );
    let he_ids: HashSet<String> = split
        .cached_hyperedges
        .iter()
        .filter_map(|h| h.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert_eq!(he_ids, ["he_ok".to_string()].into_iter().collect());
}

#[test]
#[serial]
fn save_semantic_cache_unscoped_preserves_dangling_refs_verbatim() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let doc = tmp.path().join("doc.md");
    write_text(&doc, "# Doc\n");
    let nodes = [
        json!({"id": "a", "source_file": "doc.md"}),
        json!({"id": "ghost_n", "source_file": "ghost.md"}),
    ];
    let edges = [json!({"source": "a", "target": "ghost_n", "source_file": "doc.md"})];
    let hyperedges = [json!({"id": "he", "nodes": ["a", "ghost_n"], "source_file": "doc.md"})];
    let saved = save_semantic_cache(
        &nodes,
        &edges,
        &hyperedges,
        tmp.path(),
        SemanticCacheOptions::default(),
    )
    .expect("save");
    assert_eq!(saved, 1);

    let h = file_hash(&doc, tmp.path(), None).expect("hash");
    let entry = cache_dir(tmp.path(), "semantic")
        .expect("dir")
        .join(format!("{h}.json"));
    let raw: Value =
        serde_json::from_str(&fs::read_to_string(&entry).expect("read")).expect("parse");
    assert_eq!(raw["edges"], json!(edges.to_vec()));
    assert_eq!(raw["hyperedges"], json!(hyperedges.to_vec()));
}

#[test]
#[serial]
fn save_semantic_cache_merge_existing_prunes_only_incoming() {
    _reset_stat_index_for_tests();
    let tmp = tempfile::tempdir().expect("tempdir");
    let big = tmp.path().join("big.md");
    write_text(&big, "# Big\n");
    write_text(&tmp.path().join("other.md"), "# Other\n");
    let allowed = [std::path::PathBuf::from("big.md")];

    save_semantic_cache(
        &[json!({"id": "a", "source_file": "big.md"})],
        &[json!({"source": "a", "target": "a", "relation": "self", "source_file": "big.md"})],
        &[],
        tmp.path(),
        SemanticCacheOptions {
            merge_existing: true,
            allowed_source_files: Some(&allowed),
            ..Default::default()
        },
    )
    .expect("chunk 1");
    let nodes2 = [
        json!({"id": "b", "source_file": "big.md"}),
        json!({"id": "stray", "source_file": "other.md"}),
    ];
    let edges2 = [
        json!({"source": "b", "target": "stray", "source_file": "big.md"}),
        json!({"source": "a", "target": "b", "source_file": "big.md"}),
    ];
    save_semantic_cache(
        &nodes2,
        &edges2,
        &[],
        tmp.path(),
        SemanticCacheOptions {
            merge_existing: true,
            allowed_source_files: Some(&allowed),
            ..Default::default()
        },
    )
    .expect("chunk 2");

    let cached = load_cached(&big, tmp.path(), "semantic", None).expect("cached");
    let node_ids: HashSet<String> = cached["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    assert_eq!(
        node_ids,
        ["a".to_string(), "b".to_string()].into_iter().collect()
    );
    let pairs: Vec<(String, String)> = cached["edges"]
        .as_array()
        .expect("edges")
        .iter()
        .map(|e| {
            (
                e.get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                e.get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    assert!(
        pairs.contains(&("a".to_string(), "a".to_string())),
        "prior edge survives"
    );
    assert!(
        pairs.contains(&("a".to_string(), "b".to_string())),
        "incoming valid edge kept"
    );
    assert!(!pairs.iter().any(|(s, t)| s == "stray" || t == "stray"));
}
