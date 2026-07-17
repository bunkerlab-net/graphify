//! A node whose `source_file` equals the scan root must not crash build (#1618).
//!
//! Ports the public-API cases of
//! `graphify-py/tests/test_semantic_id_remap_root.py`. `norm_source_file`
//! relativizes an absolute `source_file` equal to the root to `.`, which then fed
//! `file_stem`'s `with_extension("")` — a project-level node (`source_file` == root)
//! has no per-file identity to remap, so its id is left untouched. The private
//! `_semantic_id_remap`/`_file_stem` cases are exercised through `build_from_json`.
#![allow(clippy::expect_used)]

use graphify_build::build_from_json;
use serde_json::json;

#[test]
fn build_from_json_with_root_level_concept_node() -> Result<(), Box<dyn std::error::Error>> {
    // Previously crashed in the id-remap when a node's source_file equalled the
    // scan root (relativized to `.`). Must assemble both nodes without panicking.
    // Canonicalize so the source_file matches the root byte-for-byte where the
    // tempdir lives behind a symlink (macOS `/var` → `/private/var`).
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().canonicalize()?;
    let root_str = root.to_string_lossy().into_owned();
    let combined = json!({
        "nodes": [
            {"id": "proj_concept", "label": "Project", "file_type": "concept",
             "source_file": root_str, "_origin": "semantic"},
            {"id": "src_foo", "label": "foo", "file_type": "code",
             "source_file": "src/foo.py", "_origin": "ast"},
        ],
        "edges": [],
    });
    let g = build_from_json(combined, false, Some(&root))?;
    assert_eq!(g.node_count(), 2);
    Ok(())
}

// ── #1917: `_semantic_id_remap` must be idempotent (no id accretion) ──────────
//
// Exercised through `build_from_json` (the private `_semantic_id_remap` is not
// exported): a node's id is re-derived from its `source_file`, so the built
// graph carries the remapped id. `.claude/CLAUDE.md` has canonical stem
// `claude_claude` over legacy `claude`. Each test uses an isolated tempdir root;
// the relative `source_file` values drive the ids, so the root only anchors the
// (unused) relativisation while sandboxing filesystem access.

#[test]
fn semantic_remap_migrates_legacy_stem_once() -> Result<(), Box<dyn std::error::Error>> {
    // A pre-scheme id under `.claude/CLAUDE.md` remaps once to the canonical stem.
    let tmp = tempfile::tempdir()?;
    let ext = json!({
        "nodes": [{
            "id": "claude_graphify_trigger", "label": "trigger", "file_type": "code",
            "source_file": ".claude/CLAUDE.md", "_origin": "semantic",
        }],
        "edges": [],
    });
    let g = build_from_json(ext, false, Some(tmp.path()))?;
    assert!(
        g.contains_node("claude_claude_graphify_trigger"),
        "legacy id must migrate"
    );
    assert!(!g.contains_node("claude_graphify_trigger"));
    Ok(())
}

#[test]
fn semantic_remap_idempotent_on_canonical_stem() -> Result<(), Box<dyn std::error::Error>> {
    // An id ALREADY carrying the canonical `claude_claude` stem must not gain
    // another segment on a rebuild (#1917) — without the guard it would become
    // `claude_claude_claude_graphify_trigger`, defeating the no-change fastpath.
    let tmp = tempfile::tempdir()?;
    let ext = json!({
        "nodes": [{
            "id": "claude_claude_graphify_trigger", "label": "trigger", "file_type": "code",
            "source_file": ".claude/CLAUDE.md", "_origin": "semantic",
        }],
        "edges": [],
    });
    let g = build_from_json(ext, false, Some(tmp.path()))?;
    assert!(
        g.contains_node("claude_claude_graphify_trigger"),
        "canonical id preserved"
    );
    assert!(
        !g.contains_node("claude_claude_claude_graphify_trigger"),
        "id re-prefixed on rebuild (#1917)"
    );
    Ok(())
}

#[test]
fn semantic_remap_still_migrates_genuine_legacy_id_under_normal_path()
-> Result<(), Box<dyn std::error::Error>> {
    // The idempotency guard must not block a real one-time legacy migration.
    let tmp = tempfile::tempdir()?;
    let ext = json!({
        "nodes": [{
            "id": "readme_booking", "label": "booking", "file_type": "code",
            "source_file": "api/README.md", "_origin": "semantic",
        }],
        "edges": [],
    });
    let g = build_from_json(ext, false, Some(tmp.path()))?;
    assert!(
        g.contains_node("api_readme_booking"),
        "legacy id must migrate to canonical stem"
    );
    Ok(())
}
