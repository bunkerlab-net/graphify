//! Parity tests for `detect()` — directory walking and filtering.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — `detect()` tests.
#![allow(clippy::expect_used)]

use graphify_detect::walk::detect;
use graphify_detect::{Manifest, detect_incremental};
use tempfile::tempdir;

#[test]
fn detect_finds_python_file() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "print('hi')").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(result.files["code"].iter().any(|f| f.contains("main.py")));
}

#[test]
fn detect_memory_dir_bypasses_gitignore() -> Result<(), Box<dyn std::error::Error>> {
    // Regression for graphify-py #1047: a user ignore pattern (`*.md`) must not
    // erase the notes we generate under `graphify-out/memory`. Files inside the
    // memory sidecar bypass ignore filtering even when they match a pattern.
    let tmp = tempdir()?;
    std::fs::write(tmp.path().join(".graphifyignore"), "*.md\n")?;
    std::fs::write(tmp.path().join("main.py"), "x = 1")?;
    // A top-level markdown file is ignored…
    std::fs::write(tmp.path().join("README.md"), "# ignored")?;
    // …but a memory-dir markdown note survives.
    let mem = tmp.path().join("graphify-out").join("memory");
    std::fs::create_dir_all(&mem)?;
    std::fs::write(mem.join("note.md"), "remembered fact")?;

    let result = detect(tmp.path(), None, None);
    let docs = &result.files["document"];
    assert!(
        docs.iter().any(|f| f.contains("note.md")),
        "memory-dir note.md must be detected despite the *.md ignore rule: {docs:?}"
    );
    assert!(
        !docs.iter().any(|f| f.contains("README.md")),
        "top-level README.md must still be ignored by *.md: {docs:?}"
    );
    Ok(())
}

#[test]
fn detect_includes_code_key() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(result.files.contains_key("code"));
}

#[test]
fn detect_includes_document_key() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(result.files.contains_key("document"));
}

#[test]
fn detect_includes_video_key() {
    // detect() result always includes a 'video' key even with no video files.
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(result.files.contains_key("video"));
}

#[test]
fn detect_warns_small_corpus() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(!result.needs_graph);
    assert!(result.warning.is_some());
}

#[test]
fn detect_skips_noise_dot_dirs() {
    let tmp = tempdir().expect("tempdir");
    // Create noise dirs
    for noise_dir in [".graphify", ".next", ".nuxt", ".turbo", ".angular"] {
        let dir = tmp.path().join(noise_dir).join("cache");
        std::fs::create_dir_all(&dir).expect("create_dir_all");
        std::fs::write(dir.join("build.js"), "var s=1;").expect("test invariant");
    }
    std::fs::write(tmp.path().join("app.py"), "def go(): pass").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    for f in &all_files {
        assert!(!f.contains("/.graphify/"), "graphify cache must be skipped");
        for noise in ["/.next/", "/.nuxt/", "/.turbo/", "/.angular/"] {
            assert!(
                !f.contains(noise),
                "noise dir {noise} must be skipped, but found: {f}"
            );
        }
    }
    assert!(all_files.iter().any(|f| f.contains("app.py")));
}

#[test]
fn detect_allows_github_dir() {
    // Files inside .github/ (workflows etc.) are now indexed.
    let tmp = tempdir().expect("tempdir");
    let gh = tmp.path().join(".github").join("workflows");
    std::fs::create_dir_all(&gh).expect("create_dir_all");
    std::fs::write(
        gh.join("ci.yml"),
        "name: CI\non: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n",
    )
    .expect("test invariant");
    std::fs::write(tmp.path().join("main.py"), "def run(): pass").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    assert!(
        all_files.iter().any(|f| f.contains(".github")),
        "expected .github/workflows/ci.yml to be detected"
    );
}

#[test]
fn detect_skips_next_cache() {
    // .next/ must be excluded.
    let tmp = tempdir().expect("tempdir");
    let next_dir = tmp.path().join(".next").join("cache");
    std::fs::create_dir_all(&next_dir).expect("create_dir_all");
    std::fs::write(next_dir.join("build.js"), "(function(){var s=1;})()").expect("test invariant");
    let pages = tmp.path().join("pages");
    std::fs::create_dir_all(&pages).expect("create_dir_all");
    std::fs::write(
        pages.join("index.tsx"),
        "export default function Home() { return <div/> }",
    )
    .expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    assert!(!all_files.iter().any(|f| f.contains(".next")));
    assert!(all_files.iter().any(|f| f.contains("index.tsx")));
}

#[test]
fn detect_skips_graphify_own_cache() {
    let tmp = tempdir().expect("tempdir");
    let cache = tmp.path().join(".graphify").join("cache");
    std::fs::create_dir_all(&cache).expect("create_dir_all");
    std::fs::write(cache.join("abc123.json"), r#"{"nodes": [], "edges": []}"#)
        .expect("test invariant");
    std::fs::write(tmp.path().join("app.py"), "def go(): pass").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    assert!(!all_files.iter().any(|f| f.contains(".graphify")));
    assert!(all_files.iter().any(|f| f.contains("app.py")));
}

#[test]
fn detect_skips_coverage_dir() {
    let tmp = tempdir().expect("tempdir");
    let cov = tmp.path().join("coverage").join("lcov-report");
    std::fs::create_dir_all(&cov).expect("create_dir_all");
    std::fs::write(cov.join("index.html"), "<html>coverage report</html>").expect("test invariant");
    std::fs::write(tmp.path().join("main.py"), "def hello(): pass").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    let cov_prefix = tmp.path().join("coverage").to_string_lossy().into_owned();
    assert!(!all_files.iter().any(|f| f.starts_with(&cov_prefix)));
    assert!(all_files.iter().any(|f| f.contains("main.py")));
}

#[test]
fn detect_skips_visual_tests_dir() {
    let tmp = tempdir().expect("tempdir");
    let vt = tmp.path().join("visual-tests");
    std::fs::create_dir_all(&vt).expect("create_dir_all");
    std::fs::write(
        vt.join("bundle.js"),
        "var u3=function(){};var d2=function(){}",
    )
    .expect("test invariant");
    std::fs::write(tmp.path().join("app.py"), "def main(): pass").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    assert!(!all_files.iter().any(|f| f.contains("visual-tests")));
    assert!(all_files.iter().any(|f| f.contains("app.py")));
}

#[test]
fn detect_skips_snapshots_dir() {
    let tmp = tempdir().expect("tempdir");
    let snaps = tmp.path().join("__snapshots__");
    std::fs::create_dir_all(&snaps).expect("create_dir_all");
    std::fs::write(
        snaps.join("app.test.ts.snap"),
        "// Jest Snapshot\nexports[`test 1`] = `<div/>`",
    )
    .expect("test invariant");
    std::fs::write(
        tmp.path().join("app.ts"),
        "export function greet() { return 'hi'; }",
    )
    .expect("test invariant");
    // a bare snapshots/ dir that actually holds .snap files is still a JS artefact
    let snap = tmp.path().join("snapshots");
    std::fs::create_dir_all(&snap).expect("create_dir_all");
    std::fs::write(
        snap.join("component.test.tsx.snap"),
        "exports[`renders`] = `<span/>`",
    )
    .expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    assert!(!all_files.iter().any(|f| f.contains("__snapshots__")));
    let sep = std::path::MAIN_SEPARATOR;
    assert!(
        !all_files
            .iter()
            .any(|f| f.contains(&format!("{sep}snapshots{sep}")))
    );
    assert!(all_files.iter().any(|f| f.contains("app.ts")));
}

#[test]
fn detect_keeps_snapshots_code_namespace() {
    // #1666: a bare snapshots/ dir with NO .snap files is a legit code namespace
    // (e.g. Rails app/services/snapshots/) and must NOT be pruned as a JS artefact.
    let tmp = tempdir().expect("tempdir");
    let svc = tmp.path().join("app").join("services").join("snapshots");
    std::fs::create_dir_all(&svc).expect("create_dir_all");
    std::fs::write(
        svc.join("round_reader.rb"),
        "class RoundReader\n  def call; end\nend\n",
    )
    .expect("test invariant");
    std::fs::write(
        svc.join("backfill_marker.rb"),
        "class BackfillMarker\n  def run; end\nend\n",
    )
    .expect("test invariant");
    std::fs::write(tmp.path().join("app.rb"), "class App; end\n").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    assert!(all_files.iter().any(|f| f.contains("round_reader.rb")));
    assert!(all_files.iter().any(|f| f.contains("backfill_marker.rb")));
}

#[test]
fn detect_records_unclassified_extensionless_files() {
    // #1692: extensionless, non-shebang project files (Dockerfile, Makefile, ...)
    // were considered but left no trace. detect() now lists them under
    // `unclassified` so they can be surfaced instead of silently vanishing.
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("app.py"), "def f():\n    return 1\n").expect("test invariant");
    std::fs::write(
        tmp.path().join("Dockerfile"),
        "FROM python:3.12\nRUN pip install x\n",
    )
    .expect("test invariant");
    std::fs::write(tmp.path().join("Makefile"), "build:\n\techo hi\n").expect("test invariant");
    std::fs::write(tmp.path().join("LICENSE"), "MIT License\n").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let names: Vec<String> = result
        .unclassified
        .iter()
        .filter_map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .collect();
    assert!(names.iter().any(|n| n == "Dockerfile"), "{names:?}");
    assert!(names.iter().any(|n| n == "Makefile"), "{names:?}");
    assert!(names.iter().any(|n| n == "LICENSE"), "{names:?}");
    // classified source is NOT listed as unclassified
    assert!(!names.iter().any(|n| n == "app.py"), "{names:?}");
}

#[test]
fn detect_skips_storybook_static_dir() {
    let tmp = tempdir().expect("tempdir");
    let sb = tmp.path().join("storybook-static");
    std::fs::create_dir_all(&sb).expect("create_dir_all");
    std::fs::write(sb.join("main.js"), "(function(){var s=1;})()").expect("test invariant");
    std::fs::write(
        tmp.path().join("Button.tsx"),
        "export const Button = () => <button/>",
    )
    .expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let all_files: Vec<_> = result.files.values().flatten().collect();
    assert!(!all_files.iter().any(|f| f.contains("storybook-static")));
    assert!(all_files.iter().any(|f| f.contains("Button.tsx")));
}

#[test]
fn detect_skips_worktrees_dir() {
    let tmp = tempdir().expect("tempdir");
    let wt = tmp.path().join(".worktrees").join("feature-branch");
    std::fs::create_dir_all(&wt).expect("create_dir_all");
    std::fs::write(wt.join("main.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("app.py"), "y = 2").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let code = &result.files["code"];
    assert!(code.iter().any(|f| f.contains("app.py")));
    assert!(!code.iter().any(|f| f.contains(".worktrees")));
}

#[test]
fn detect_skips_nested_worktrees_dir() {
    // graphify-py #1023: files inside `.claude/worktrees/` (nested placement
    // within a dotted parent) are never indexed.
    let tmp = tempdir().expect("tempdir");
    let wt = tmp
        .path()
        .join(".claude")
        .join("worktrees")
        .join("feature-branch");
    std::fs::create_dir_all(&wt).expect("create_dir_all");
    std::fs::write(wt.join("main.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("app.py"), "y = 2").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let code = &result.files["code"];
    assert!(code.iter().any(|f| f.contains("app.py")));
    assert!(!code.iter().any(|f| f.contains("worktrees")));
}

#[test]
fn detect_graphifyignore_excludes_file() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(".graphifyignore"),
        "vendor/\n*.generated.py\n",
    )
    .expect("test invariant");
    let vendor = tmp.path().join("vendor");
    std::fs::create_dir_all(&vendor).expect("create_dir_all");
    std::fs::write(vendor.join("lib.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("main.py"), "print('hi')").expect("test invariant");
    std::fs::write(tmp.path().join("schema.generated.py"), "x = 1").expect("test invariant");

    let result = detect(tmp.path(), None, None);
    let code = &result.files["code"];
    assert!(code.iter().any(|f| f.contains("main.py")));
    assert!(!code.iter().any(|f| f.contains("vendor")));
    assert!(!code.iter().any(|f| f.contains("generated")));
    assert_eq!(result.graphifyignore_patterns, 2);
}

#[test]
fn detect_graphifyignore_missing_is_fine() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert_eq!(result.graphifyignore_patterns, 0);
}

#[test]
fn detect_graphifyignore_comments_ignored() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join(".graphifyignore"),
        "# this is a comment\n\nmain.py\n",
    )
    .expect("test invariant");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("other.py"), "x = 2").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(!result.files["code"].iter().any(|f| f.contains("main.py")));
    assert!(result.files["code"].iter().any(|f| f.contains("other.py")));
}

#[cfg(unix)]
#[test]
fn detect_follows_symlinked_directory() {
    let tmp = tempdir().expect("tempdir");
    let real_dir = tmp.path().join("real_lib");
    std::fs::create_dir_all(&real_dir).expect("create_dir_all");
    std::fs::write(real_dir.join("util.py"), "x = 1").expect("test invariant");
    std::os::unix::fs::symlink(&real_dir, tmp.path().join("linked_lib")).expect("test invariant");

    let result_no = detect(tmp.path(), Some(false), None);
    let result_yes = detect(tmp.path(), Some(true), None);

    assert!(
        result_no.files["code"]
            .iter()
            .any(|f| f.contains("real_lib"))
    );
    assert!(
        !result_no.files["code"]
            .iter()
            .any(|f| f.contains("linked_lib"))
    );
    assert!(
        result_yes.files["code"]
            .iter()
            .any(|f| f.contains("linked_lib"))
    );
}

#[cfg(unix)]
#[test]
fn detect_follows_symlinked_file() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("real.py"), "x = 1").expect("test invariant");
    std::os::unix::fs::symlink(tmp.path().join("real.py"), tmp.path().join("link.py"))
        .expect("test invariant");
    let result = detect(tmp.path(), Some(true), None);
    let code = &result.files["code"];
    assert!(code.iter().any(|f| f.contains("real.py")));
    assert!(code.iter().any(|f| f.contains("link.py")));
}

#[cfg(unix)]
#[test]
fn detect_handles_circular_symlinks() {
    let tmp = tempdir().expect("tempdir");
    let sub = tmp.path().join("a");
    std::fs::create_dir_all(&sub).expect("create_dir_all");
    std::fs::write(sub.join("main.py"), "x = 1").expect("test invariant");
    std::os::unix::fs::symlink(tmp.path(), sub.join("loop")).expect("test invariant");
    let result = detect(tmp.path(), Some(true), None);
    assert!(result.files["code"].iter().any(|f| f.contains("main.py")));
}

#[cfg(unix)]
#[test]
fn detect_does_not_auto_follow_symlink_child() {
    // 009a98b: following symlinks is now an explicit opt-in — a symlinked child no
    // longer auto-enables it. The real directory is still scanned directly; the
    // symlink is only traversed when `follow_symlinks = Some(true)` and its target
    // stays inside the scan root.
    let tmp = tempdir().expect("tempdir");
    let real_dir = tmp.path().join("real_lib");
    std::fs::create_dir_all(&real_dir).expect("create_dir_all");
    std::fs::write(real_dir.join("util.py"), "x = 1").expect("test invariant");
    std::os::unix::fs::symlink(&real_dir, tmp.path().join("linked_lib")).expect("test invariant");

    // Default (no kwarg): symlink NOT followed, but the real dir is scanned.
    let default_run = detect(tmp.path(), None, None);
    assert!(
        default_run.files["code"]
            .iter()
            .any(|f| f.contains("real_lib")),
        "the real directory must still be scanned"
    );
    assert!(
        !default_run.files["code"]
            .iter()
            .any(|f| f.contains("linked_lib")),
        "the symlinked child must NOT be auto-followed (009a98b)"
    );

    // Explicit opt-in: the in-root symlink target is followed.
    let opt_in = detect(tmp.path(), Some(true), None);
    assert!(
        opt_in.files["code"]
            .iter()
            .any(|f| f.contains("linked_lib")),
        "an explicitly-followed in-root symlink must be scanned"
    );
}

#[test]
fn detect_default_does_not_follow_when_no_symlinks() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    let sub = tmp.path().join("sub");
    std::fs::create_dir_all(&sub).expect("create_dir_all");
    std::fs::write(sub.join("other.py"), "y = 2").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(result.files["code"].iter().any(|f| f.contains("main.py")));
    assert!(result.files["code"].iter().any(|f| f.contains("other.py")));
}

#[cfg(unix)]
#[test]
fn detect_explicit_false_overrides_auto_detect() {
    let tmp = tempdir().expect("tempdir");
    let real_dir = tmp.path().join("real_lib");
    std::fs::create_dir_all(&real_dir).expect("create_dir_all");
    std::fs::write(real_dir.join("util.py"), "x = 1").expect("test invariant");
    std::os::unix::fs::symlink(&real_dir, tmp.path().join("linked_lib")).expect("test invariant");
    // Explicit false overrides auto-detect; symlink contents must NOT appear.
    let result = detect(tmp.path(), Some(false), None);
    assert!(
        !result.files["code"]
            .iter()
            .any(|f| f.contains("linked_lib"))
    );
}

#[test]
fn detect_finds_video_files() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("lecture.mp4"), b"fake video data").expect("test invariant");
    std::fs::write(tmp.path().join("notes.md"), "# Notes\nSome content here.")
        .expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert_eq!(result.files["video"].len(), 1);
    assert!(
        result.files["video"]
            .iter()
            .any(|f| f.contains("lecture.mp4"))
    );
}

#[test]
fn detect_video_not_in_words() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("clip.mp4"), vec![0u8; 100]).expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert_eq!(result.total_words, 0);
}

#[test]
fn detect_skips_google_workspace_shortcuts_by_default() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("notes.gdoc"), r#"{"doc_id":"doc-1"}"#).expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(result.files["document"].is_empty());
    assert!(
        result
            .skipped_sensitive
            .iter()
            .any(|item| item.contains("Google Workspace shortcut skipped"))
    );
}

#[test]
fn detect_extra_excludes_pattern() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("secret.py"), "API_KEY = 'abc'").expect("test invariant");
    let subdir = tmp.path().join("legacy");
    std::fs::create_dir_all(&subdir).expect("create_dir_all");
    std::fs::write(subdir.join("old.py"), "y = 2").expect("test invariant");
    let result = detect(
        tmp.path(),
        None,
        Some(&["secret.py".to_string(), "legacy/".to_string()]),
    );
    let code = &result.files["code"];
    assert!(code.iter().any(|f| f.contains("main.py")));
    assert!(!code.iter().any(|f| f.contains("secret.py")));
    assert!(!code.iter().any(|f| f.contains("legacy")));
}

#[test]
fn detect_gitignore_fallback_when_no_graphifyignore() {
    // When no .graphifyignore exists, .gitignore patterns are honored.
    let tmp = tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join(".git")).expect("test invariant");
    std::fs::write(tmp.path().join(".gitignore"), "vendor/\n*.generated.py\n")
        .expect("test invariant");
    let vendor = tmp.path().join("vendor");
    std::fs::create_dir_all(&vendor).expect("create_dir_all");
    std::fs::write(vendor.join("lib.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("main.py"), "print('hi')").expect("test invariant");
    std::fs::write(tmp.path().join("schema.generated.py"), "x = 1").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let code = &result.files["code"];
    assert!(code.iter().any(|f| f.contains("main.py")));
    assert!(!code.iter().any(|f| f.contains("vendor")));
    assert!(!code.iter().any(|f| f.contains("generated")));
}

#[test]
fn detect_graphifyignore_and_gitignore_are_merged() {
    // #1363: when both exist, their patterns are MERGED — a file excluded only
    // by .gitignore stays excluded even though .graphifyignore says nothing
    // about it. Previously a .graphifyignore silently disabled the dir's
    // .gitignore, leaking gitignore-only secrets into the graph.
    let tmp = tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join(".git")).expect("test invariant");
    std::fs::write(tmp.path().join(".gitignore"), "main.py\n").expect("test invariant");
    std::fs::write(tmp.path().join(".graphifyignore"), "other.py\n").expect("test invariant");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("other.py"), "x = 2").expect("test invariant");
    std::fs::write(tmp.path().join("keep.py"), "x = 3").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let code = &result.files["code"];
    assert!(!code.iter().any(|f| f.contains("main.py"))); // gitignore STILL applied
    assert!(!code.iter().any(|f| f.contains("other.py"))); // graphifyignore applied
    assert!(code.iter().any(|f| f.contains("keep.py"))); // neither excludes it
}

#[test]
fn detect_graphifyignore_negation_overrides_gitignore() {
    // #1363: .graphifyignore is evaluated after .gitignore, so a `!` negation in
    // it re-includes a file the .gitignore excluded (last-match-wins).
    let tmp = tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join(".git")).expect("test invariant");
    std::fs::write(tmp.path().join(".gitignore"), "*.py\n").expect("test invariant");
    std::fs::write(tmp.path().join(".graphifyignore"), "!keep.py\n").expect("test invariant");
    std::fs::write(tmp.path().join("main.py"), "x = 1").expect("test invariant");
    std::fs::write(tmp.path().join("keep.py"), "x = 2").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    let code = &result.files["code"];
    assert!(code.iter().any(|f| f.contains("keep.py"))); // rescued by negation
    assert!(!code.iter().any(|f| f.contains("main.py"))); // still excluded
}

// ── #1276: a single `!` negation must not disable directory pruning ──────────

#[test]
fn negation_does_not_disable_directory_pruning() {
    // `!docs/**` must not switch off pruning of the unrelated ignored `myignored/`
    // dir. Output stays correct either way; this guards that the negation still
    // re-includes docs/*.md, real source is found, and nothing leaks out of the
    // ignored dir.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(root.join(".graphifyignore"), "myignored/\n*.md\n!docs/**\n").expect("write");
    let deep = root.join("myignored").join("deep").join("deeper");
    std::fs::create_dir_all(&deep).expect("mkdir deep");
    std::fs::write(deep.join("junk.py"), "x = 1").expect("write junk");
    std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    std::fs::write(root.join("docs").join("guide.md"), "# guide").expect("write guide");
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(root.join("src").join("app.py"), "y = 2").expect("write app");

    let result = detect(root, None, None);
    let all_files: Vec<&String> = result.files.values().flatten().collect();
    assert!(all_files.iter().any(|p| p.ends_with("app.py")));
    assert!(all_files.iter().any(|p| p.ends_with("guide.md")));
    assert!(
        !all_files.iter().any(|p| p.contains("junk.py")),
        "junk.py under ignored myignored/ must not leak out"
    );
}

// ── #1163: detect_incremental must survive a dict-valued mtime ───────────────

#[test]
fn detect_incremental_survives_dict_valued_mtime() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    let src = root.join("mod.py");
    std::fs::write(&src, "def f():\n    return 1\n").expect("write src");

    let manifest_dir = root.join("graphify-out");
    std::fs::create_dir_all(&manifest_dir).expect("mkdir");
    let abs_key = src
        .canonicalize()
        .expect("canon")
        .to_string_lossy()
        .into_owned();
    // Drifted entry: mtime stored as a nested dict rather than a float.
    let drifted = serde_json::json!({
        abs_key: {
            "mtime": {"mtime": 123.0},
            "ast_hash": "deadbeefdeadbeefdeadbeefdeadbeef",
            "semantic_hash": "cafebabecafebabecafebabecafebabe",
        }
    });
    std::fs::write(
        manifest_dir.join("manifest.json"),
        serde_json::to_string(&drifted).expect("ser"),
    )
    .expect("write manifest");

    // Must not panic; the drifted file is re-verified by hash and treated as new.
    let result = detect_incremental(root, &Manifest::new()).expect("incremental");
    let changed: Vec<&String> = result.changed_files.values().flatten().collect();
    let unchanged: Vec<&String> = result.unchanged_files.values().flatten().collect();
    assert!(changed.iter().any(|f| f.contains("mod.py")));
    assert!(!unchanged.iter().any(|f| f.contains("mod.py")));
}
#[test]
fn detect_reports_walk_errors_key() {
    // detect() always surfaces a walk_errors list so callers can tell whether
    // enumeration was complete.
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("a.py"), "def f(): pass\n").expect("test invariant");
    let result = detect(tmp.path(), None, None);
    assert!(
        result.walk_errors.is_empty(),
        "clean walk must report no errors: {:?}",
        result.walk_errors
    );
}

#[cfg(unix)]
#[test]
fn detect_surfaces_unreadable_dir_instead_of_silent_skip() {
    // A subtree whose scan raises (permissions, or a dir deleted mid-walk) used to
    // be silently dropped, yielding a partial graph. detect() now records it in
    // walk_errors while still enumerating the rest of the tree.
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("a.py"), "def f(): pass\n").expect("test invariant");
    let locked = tmp.path().join("locked");
    std::fs::create_dir(&locked).expect("mkdir");
    std::fs::write(locked.join("b.py"), "def g(): pass\n").expect("test invariant");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    // If the perms don't actually block the scan (running as root, or a platform
    // that ignores mode 0o000), the skip path can't be exercised — bail cleanly.
    if std::fs::read_dir(&locked).is_ok() {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).ok();
        return;
    }

    let result = detect(tmp.path(), None, None);
    // Restore perms so the tempdir can be cleaned up.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).ok();

    assert!(
        result.files["code"].iter().any(|f| f.ends_with("a.py")),
        "the rest of the tree must still be enumerated: {:?}",
        result.files["code"]
    );
    assert!(
        !result.walk_errors.is_empty(),
        "the unreadable directory must be recorded in walk_errors"
    );
}
