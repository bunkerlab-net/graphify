//! Parity tests for issue #1094: `to_obsidian` / `to_canvas` must cap filenames
//! to stay under the 255-byte filesystem limit, instead of crashing with
//! `OSError` ENAMETOOLONG on long node labels.
//!
//! Mirrors `graphify-py/tests/test_obsidian_filename_cap.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use graphify_build::{Graph, build_from_json};
use graphify_export::{to_canvas, to_obsidian};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Value, json};
use tempfile::tempdir;

/// Build a chain graph (each node wikilinks to the next) with the given labels,
/// all in community 0. Mirrors the Python `_graph` helper.
fn graph(labels: &[&str]) -> (Graph, IndexMap<i64, Vec<String>>) {
    let mut nodes = Vec::new();
    let mut ids = Vec::new();
    for (i, lab) in labels.iter().enumerate() {
        let nid = format!("n{i}");
        nodes.push(json!({
            "id": nid,
            "label": lab,
            "file_type": "code",
            "source_file": "x.py",
            "community": 0,
        }));
        ids.push(nid);
    }
    let mut edges = Vec::new();
    for w in ids.windows(2) {
        edges.push(json!({
            "source": w[0],
            "target": w[1],
            "relation": "calls",
            "confidence": "EXTRACTED",
        }));
    }
    let g =
        build_from_json(json!({"nodes": nodes, "edges": edges}), false, None).expect("build graph");
    let mut comms: IndexMap<i64, Vec<String>> = IndexMap::new();
    comms.insert(0, ids);
    (g, comms)
}

/// Largest `*.md` filename in `dir`, measured in UTF-8 bytes.
fn max_name_bytes(dir: &Path) -> usize {
    fs::read_dir(dir)
        .expect("read_dir")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension() == Some(OsStr::new("md")))
        .map(|e| e.file_name().as_encoded_bytes().len())
        .max()
        .unwrap_or(0)
}

#[test]
fn obsidian_long_ascii_label_does_not_crash() {
    let (g, comms) = graph(&[&"a".repeat(300), "short"]);
    let tmp = tempdir().expect("tempdir");
    to_obsidian(&g, &comms, tmp.path(), None, None).expect("to_obsidian");
    assert!(max_name_bytes(tmp.path()) <= 255);
}

#[test]
fn obsidian_long_cjk_label_byte_cap() {
    // 300 CJK chars = 900 bytes in UTF-8: a char cap would still overflow.
    let (g, comms) = graph(&[&"中".repeat(300), "ok"]);
    let tmp = tempdir().expect("tempdir");
    to_obsidian(&g, &comms, tmp.path(), None, None).expect("to_obsidian");
    assert!(max_name_bytes(tmp.path()) <= 255);
}

#[test]
fn obsidian_distinct_long_labels_sharing_prefix_do_not_collide() {
    let prefix = "z".repeat(250);
    let (g, comms) = graph(&[&format!("{prefix}_ALPHA"), &format!("{prefix}_BETA")]);
    let tmp = tempdir().expect("tempdir");
    to_obsidian(&g, &comms, tmp.path(), None, None).expect("to_obsidian");

    let md_files: Vec<_> = fs::read_dir(tmp.path())
        .expect("read_dir")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension() == Some(OsStr::new("md")))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with("_COMMUNITY_"))
        .collect();
    // Two distinct nodes must produce two distinct files (no overwrite).
    assert_eq!(md_files.len(), 2, "{md_files:?}");
    assert!(max_name_bytes(tmp.path()) <= 255);
}

#[test]
fn obsidian_wikilink_resolves_after_truncation() {
    let (g, comms) = graph(&[&"w".repeat(300), "neighbor"]);
    let tmp = tempdir().expect("tempdir");
    to_obsidian(&g, &comms, tmp.path(), None, None).expect("to_obsidian");

    // The note for "neighbor" should link to the truncated filename of the long
    // label. Every [[target]] must correspond to a real .md file on disk.
    let neighbor_note = fs::read_to_string(tmp.path().join("neighbor.md")).expect("neighbor.md");
    let re = Regex::new(r"\[\[([^\]]+)\]\]").expect("valid regex");
    let targets: Vec<&str> = re
        .captures_iter(&neighbor_note)
        .map(|c| c.get(1).expect("group 1").as_str())
        .collect();
    assert!(!targets.is_empty(), "no wikilink found in neighbor note");
    for t in targets {
        assert!(
            tmp.path().join(format!("{t}.md")).exists(),
            "dangling wikilink: {t}"
        );
    }
}

#[test]
fn canvas_long_label_file_ref_capped() {
    let (g, comms) = graph(&[&"c".repeat(300), "ok"]);
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &comms, &out, None, None).expect("to_canvas");
    let data: Value = serde_json::from_str(&fs::read_to_string(&out).expect("read canvas"))
        .expect("parse canvas");
    for node in data
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if node.get("type").and_then(Value::as_str) == Some("file") {
            let file = node
                .get("file")
                .and_then(Value::as_str)
                .expect("file field");
            assert!(file.len() <= 255, "{file}");
        }
    }
}
