//! Parity tests against `graphify-py/tests/test_cache.py`.
#![allow(clippy::expect_used, unsafe_code)]

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use graphify_cache::{
    _reset_stat_index_for_tests, body_content, cache_dir, cache_dir_versioned, cached_files,
    cached_word_count, clear_cache, ensure_atexit_flush_registered, file_hash, flush_stat_index,
    load_cached, load_cached_versioned, prune_semantic_cache, save_cached, save_cached_versioned,
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
    write_text(&f, "completely different content");
    bump_mtime(&f);
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
    write_text(
        &f,
        "---\nreviewed: 2026-04-09\n---\n\n# Title\n\nBody text.",
    );
    bump_mtime(&f);
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
    write_text(
        &f,
        "---\nreviewed: 2026-01-01\n---\n\n# Title\n\nChanged body.",
    );
    bump_mtime(&f);
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
    write_text(&f, "# Just a heading\n\nDifferent content.");
    bump_mtime(&f);
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
    write_text(&f, "# changed comment\nx = 1");
    bump_mtime(&f);
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

// ── ensure_atexit_flush_registered ───────────────────────────────────────────

#[test]
fn ensure_atexit_flush_registered_is_idempotent() {
    // Calling multiple times must not panic or have visible side-effects.
    ensure_atexit_flush_registered();
    ensure_atexit_flush_registered();
    ensure_atexit_flush_registered();
    // No assertion needed beyond "did not panic".
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
    let h1 = file_hash(&f, tmp.path()).expect("hash");
    bump_mtime(&f);
    write_text(&f, "----\nEdited intro paragraph.\n\n---\nbody");
    let h2 = file_hash(&f, tmp.path()).expect("hash");
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
    save_cached(&src, &result, root, "ast").expect("save");

    let h = file_hash(&src, root).expect("hash");
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
    )
    .expect("save");

    let loaded = load_cached(&src, root, "ast").expect("loaded");
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
    let h = file_hash(&src, root).expect("hash");
    let entry = cache_dir(root, "ast")
        .expect("dir")
        .join(format!("{h}.json"));
    let payload = serde_json::to_string(
        &json!({"nodes": [{"id": "n1", "source_file": abs_src.clone()}], "edges": []}),
    )
    .expect("ser");
    fs::write(&entry, payload).expect("write");

    let loaded = load_cached(&src, root, "ast").expect("loaded");
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
    )
    .expect("save");

    // Copy corpus + cache to a second location with a different prefix.
    let repo_b = tmp.path().join("repo_b");
    copy_dir_all(&repo_a, &repo_b);

    let src_b = repo_b.join("src").join("foo.py");
    let loaded = load_cached(&src_b, &repo_b, "ast").expect("portable");
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
    )
    .expect("save");
    assert!(load_cached_versioned(&f, root, "ast", "0.8.0").is_some());
    assert!(
        load_cached_versioned(&f, root, "ast", "0.8.1").is_none(),
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
    let h = file_hash(&f, root).expect("hash");
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

    assert!(load_cached(&f, root, "ast").is_none());
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
    )
    .expect("save");
    let semantic_dir = cache_dir(root, "semantic").expect("semantic dir");

    // A version bump triggers AST cleanup; the semantic cache must survive.
    cache_dir_versioned(root, "ast", "0.8.1").expect("bump");
    assert!(load_cached_versioned(&f, root, "semantic", "0.8.1").is_some());
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
    )
    .expect("save");

    let h = file_hash(&alias, root).expect("hash");
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
    let h_a = file_hash(&f, tmp.path()).expect("h_a");
    save_cached(
        &f,
        &json!({"nodes": [{"id": "a"}], "edges": []}),
        tmp.path(),
        "semantic",
    )
    .expect("save a");

    write_text(&f, "# B\n\nContent B.\n");
    _reset_stat_index_for_tests();
    bump_mtime(&f);
    let h_b = file_hash(&f, tmp.path()).expect("h_b");
    save_cached(
        &f,
        &json!({"nodes": [{"id": "b"}], "edges": []}),
        tmp.path(),
        "semantic",
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
        )
        .expect("save");
        live_hashes.insert(file_hash(&f, tmp.path()).expect("hash"));
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
    let h = file_hash(&f, tmp.path()).expect("hash");
    save_cached(
        &f,
        &json!({"nodes": [{"id": "g"}], "edges": []}),
        tmp.path(),
        "semantic",
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
        false,
    )
    .expect("save 1");
    save_semantic_cache(
        &[json!({"id": "b", "source_file": "doc.md"})],
        &[],
        &[],
        tmp.path(),
        false,
    )
    .expect("save 2");
    let cached = load_cached(&f, tmp.path(), "semantic").expect("cached");
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
        true,
    )
    .expect("chunk 1");
    save_semantic_cache(
        &[json!({"id": "b", "source_file": "big.md"})],
        &[json!({"source": "b", "target": "y", "source_file": "big.md"})],
        &[json!({"id": "h2", "nodes": ["b"], "source_file": "big.md"})],
        tmp.path(),
        true,
    )
    .expect("chunk 2");
    let cached = load_cached(&f, tmp.path(), "semantic").expect("cached");
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
    let h = file_hash(&f, tmp.path()).expect("hash");
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
    assert_eq!(file_hash(&f, tmp.path()).expect("hash"), h);
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
