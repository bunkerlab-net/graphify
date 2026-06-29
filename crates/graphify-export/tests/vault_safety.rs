//! Parity tests for Obsidian vault safety (#1506), the canvas sqrt(n) grid
//! (#1452), and case-fold filename dedup (#1453), ported from
//! `graphify-py/tests/test_export.py`.
// The single-char loop indices and by-value `json!` helpers read naturally in
// these fixtures.
#![allow(clippy::many_single_char_names, clippy::needless_pass_by_value)]

use std::path::Path;

use graphify_build::{Graph, build_from_json};
use graphify_cluster::cluster;
use graphify_export::{to_canvas, to_obsidian};
use indexmap::IndexMap;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

// Fixture builder: `expect` on a known-good in-test JSON literal is the clearest
// failure signal here, so this one helper keeps the narrow allow.
#[allow(clippy::expect_used)]
fn build(nodes: Value, edges: Value) -> Graph {
    build_from_json(json!({ "nodes": nodes, "edges": edges }), false, None).expect("build")
}

fn two_node_graph() -> (Graph, IndexMap<i64, Vec<String>>) {
    let g = build(
        json!([
            {"id": "n1", "label": "Database", "file_type": "code", "source_file": "app/db.py"},
            {"id": "n2", "label": "Server", "file_type": "code", "source_file": "app/srv.py"},
        ]),
        json!([{"source": "n1", "target": "n2", "relation": "calls",
                "confidence": "EXTRACTED", "source_file": "app/db.py"}]),
    );
    let communities: IndexMap<i64, Vec<String>> =
        IndexMap::from([(0, vec!["n1".to_string(), "n2".to_string()])]);
    (g, communities)
}

fn case_collision_graph() -> Graph {
    build(
        json!([
            {"id": "n1", "label": "References", "file_type": "code", "source_file": "a.py"},
            {"id": "n2", "label": "references", "file_type": "document", "source_file": "b.md"},
        ]),
        json!([]),
    )
}

fn md_node_notes(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("md")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                && !stem.starts_with("_COMMUNITY")
            {
                out.push(stem.to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out
}

#[test]
// JSON navigation inside `.map`/`.find` closures can't use `?`, so this
// assertion-dense canvas-grid test keeps the narrow allow.
#[allow(clippy::unwrap_used)]
fn to_canvas_node_grid_matches_box_columns() -> TestResult {
    // #1452: cards lay out in the ceil(sqrt(n))-column / ceil(n/cols)-row grid the
    // box is sized for. Covers a perfect square (25 -> 5x5) and a non-square (10).
    for n in [10usize, 25] {
        let nodes: Vec<Value> = (0..n)
            .map(|i| {
                json!({"id": format!("n{i}"), "label": format!("sym_{i:02}"),
                            "file_type": "code", "source_file": "a.py"})
            })
            .collect();
        let g = build(json!(nodes), json!([]));
        let communities: IndexMap<i64, Vec<String>> =
            IndexMap::from([(0, (0..n).map(|i| format!("n{i}")).collect())]);
        let tmp = tempfile::tempdir()?;
        let out = tmp.path().join("graph.canvas");
        to_canvas(&g, &communities, &out, None, None)?;
        let data: Value = serde_json::from_str(&std::fs::read_to_string(&out)?)?;
        let canvas_nodes = data["nodes"].as_array().unwrap();
        let group = canvas_nodes.iter().find(|c| c["type"] == "group").unwrap();
        let cards: Vec<&Value> = canvas_nodes
            .iter()
            .filter(|c| c["type"] == "file")
            .collect();
        assert_eq!(cards.len(), n, "n={n}");

        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let expected_cols = (n as f64).sqrt().ceil() as usize;
        let expected_rows = n.div_ceil(expected_cols);
        let distinct_x = cards
            .iter()
            .map(|c| c["x"].as_i64().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let distinct_y = cards
            .iter()
            .map(|c| c["y"].as_i64().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(distinct_x, expected_cols, "n={n}: cols");
        assert_eq!(distinct_y, expected_rows, "n={n}: rows");

        let (gx, gy, gw, gh) = (
            group["x"].as_i64().unwrap(),
            group["y"].as_i64().unwrap(),
            group["width"].as_i64().unwrap(),
            group["height"].as_i64().unwrap(),
        );
        for c in &cards {
            let (x, y, w, h) = (
                c["x"].as_i64().unwrap(),
                c["y"].as_i64().unwrap(),
                c["width"].as_i64().unwrap(),
                c["height"].as_i64().unwrap(),
            );
            assert!(gx <= x && x + w <= gx + gw, "n={n} card x out of box");
            assert!(gy <= y && y + h <= gy + gh, "n={n} card y out of box");
        }
    }
    Ok(())
}

#[test]
fn to_obsidian_preserves_existing_user_notes_and_obsidian_config() -> TestResult {
    let (g, communities) = two_node_graph();
    let labels: IndexMap<i64, String> = IndexMap::from([(0, "Backend".to_string())]);
    let tmp = tempfile::tempdir()?;
    let vault = tmp.path();
    std::fs::write(vault.join("Database.md"), "# MY NOTES\nkeep me\n")?;
    std::fs::create_dir(vault.join(".obsidian"))?;
    std::fs::write(
        vault.join(".obsidian/graph.json"),
        "{\"USER\":\"settings\"}",
    )?;
    to_obsidian(&g, &communities, vault, Some(&labels), None)?;
    assert!(std::fs::read_to_string(vault.join("Database.md"))?.contains("MY NOTES"));
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(
        vault.join(".obsidian/graph.json"),
    )?)?;
    assert_eq!(cfg, json!({"USER": "settings"}));
    assert!(vault.join("Server.md").exists());
    Ok(())
}

#[test]
fn to_obsidian_empty_dir_writes_full_vault() -> TestResult {
    let (g, communities) = two_node_graph();
    let labels: IndexMap<i64, String> = IndexMap::from([(0, "Backend".to_string())]);
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("obsidian");
    let n = to_obsidian(&g, &communities, &out, Some(&labels), None)?;
    assert!(out.join("Database.md").exists() && out.join("Server.md").exists());
    assert!(out.join(".obsidian/graph.json").exists());
    assert_eq!(n, 3); // 2 node notes + 1 community note
    Ok(())
}

#[test]
fn to_obsidian_rerun_updates_own_notes_but_not_user_files() -> TestResult {
    let (g, communities) = two_node_graph();
    let l1: IndexMap<i64, String> = IndexMap::from([(0, "Backend".to_string())]);
    let l2: IndexMap<i64, String> = IndexMap::from([(0, "Backend2".to_string())]);
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("obsidian");
    to_obsidian(&g, &communities, &out, Some(&l1), None)?;
    std::fs::write(out.join("UserNote.md"), "mine\n")?;
    to_obsidian(&g, &communities, &out, Some(&l2), None)?;
    assert!(out.join("Database.md").exists()); // graphify re-wrote its own
    assert_eq!(
        std::fs::read_to_string(out.join("UserNote.md"))?.trim(),
        "mine"
    );
    Ok(())
}

#[test]
fn to_obsidian_case_only_distinct_labels_dont_overwrite() -> TestResult {
    let g = case_collision_graph();
    let communities = cluster(&g, 1.0, None);
    let tmp = tempfile::tempdir()?;
    to_obsidian(&g, &communities, tmp.path(), None, None)?;
    let mut notes = md_node_notes(tmp.path());
    assert_eq!(notes.len(), g.node_count(), "{notes:?}");
    let lowered: std::collections::HashSet<String> =
        notes.iter().map(|s| s.to_lowercase()).collect();
    assert_eq!(lowered.len(), notes.len(), "{notes:?}");
    notes.sort();
    assert_eq!(
        notes,
        vec!["References".to_string(), "references_1".to_string()]
    );
    Ok(())
}

#[test]
fn to_obsidian_generated_suffix_doesnt_overwrite_literal() -> TestResult {
    let g = build(
        json!([
            {"id": "a", "label": "dup", "file_type": "code", "source_file": "a.py"},
            {"id": "b", "label": "dup", "file_type": "code", "source_file": "b.py"},
            {"id": "c", "label": "dup_1", "file_type": "code", "source_file": "c.py"},
        ]),
        json!([]),
    );
    let communities = cluster(&g, 1.0, None);
    let tmp = tempfile::tempdir()?;
    to_obsidian(&g, &communities, tmp.path(), None, None)?;
    let notes = md_node_notes(tmp.path());
    assert_eq!(notes.len(), 3, "{notes:?}");
    let lowered: std::collections::HashSet<String> =
        notes.iter().map(|s| s.to_lowercase()).collect();
    assert_eq!(lowered.len(), 3, "{notes:?}");
    Ok(())
}

#[test]
// `.map` closures over the canvas JSON can't use `?`, so this keeps the narrow allow.
#[allow(clippy::unwrap_used)]
fn to_canvas_case_only_distinct_labels_get_distinct_files() -> TestResult {
    let g = case_collision_graph();
    let communities = cluster(&g, 1.0, None);
    let tmp = tempfile::tempdir()?;
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &communities, &out, None, None)?;
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&out)?)?;
    let files: Vec<String> = data["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "file")
        .map(|c| c["file"].as_str().unwrap().to_lowercase())
        .collect();
    assert_eq!(
        files.len(),
        g.node_count(),
        "a colliding card was dropped: {files:?}"
    );
    let distinct: std::collections::HashSet<&String> = files.iter().collect();
    assert_eq!(distinct.len(), files.len(), "{files:?}");
    Ok(())
}

#[test]
// `.map` closures over the canvas JSON can't use `?`, so this keeps the narrow allow.
#[allow(clippy::unwrap_used)]
fn obsidian_canvas_filenames_agree() -> TestResult {
    let g = case_collision_graph();
    let communities = cluster(&g, 1.0, None);
    let tmp = tempfile::tempdir()?;
    to_obsidian(&g, &communities, tmp.path(), None, None)?;
    let note_stems: std::collections::HashSet<String> =
        md_node_notes(tmp.path()).into_iter().collect();
    let out = tmp.path().join("graph.canvas");
    to_canvas(&g, &communities, &out, None, None)?;
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&out)?)?;
    let canvas_stems: std::collections::HashSet<String> = data["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "file")
        .map(|c| {
            Path::new(c["file"].as_str().unwrap())
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        canvas_stems, note_stems,
        "{canvas_stems:?} != {note_stems:?}"
    );
    Ok(())
}

#[test]
fn to_obsidian_community_notes_case_collision() -> TestResult {
    let g = build(
        json!([
            {"id": "n1", "label": "alpha", "file_type": "code", "source_file": "a.py"},
            {"id": "n2", "label": "beta", "file_type": "code", "source_file": "b.py"},
        ]),
        json!([]),
    );
    let communities: IndexMap<i64, Vec<String>> =
        IndexMap::from([(0, vec!["n1".to_string()]), (1, vec!["n2".to_string()])]);
    let labels: IndexMap<i64, String> =
        IndexMap::from([(0, "API".to_string()), (1, "Api".to_string())]);
    let tmp = tempfile::tempdir()?;
    to_obsidian(&g, &communities, tmp.path(), Some(&labels), None)?;
    let comm: Vec<String> = std::fs::read_dir(tmp.path())?
        .flatten()
        .filter_map(|e| {
            let stem = e.path().file_stem()?.to_string_lossy().into_owned();
            (stem.starts_with("_COMMUNITY_")).then_some(stem)
        })
        .collect();
    assert_eq!(comm.len(), 2, "{comm:?}");
    let lowered: std::collections::HashSet<String> =
        comm.iter().map(|s| s.to_lowercase()).collect();
    assert_eq!(lowered.len(), comm.len(), "{comm:?}");
    Ok(())
}
