//! Small helpers used by the rebuild pipeline.
//!
//! Extracted from `rebuild.rs` to keep the pipeline module focused on
//! sequencing and leave utility functions here.

use std::path::{Path, PathBuf};

use graphify_analyze::{god_nodes, suggest_questions, surprising_connections};
use graphify_build::Graph;
use graphify_detect::{DetectResult, FileType, classify_file, detect};
use indexmap::IndexMap;
use serde_json::{Value, json};

// ── report_root_label ─────────────────────────────────────────────────────────

/// Return the display label for the project root used in reports.
///
/// Ports `_report_root_label` from `watch.py:125-128`.
pub(crate) fn report_root_label(watch_path: &Path) -> String {
    if watch_path.is_absolute() {
        watch_path
            .file_name()
            .and_then(|n| n.to_str())
            .map_or_else(|| watch_path.to_string_lossy().into_owned(), str::to_string)
    } else if watch_path == Path::new(".") {
        std::env::current_dir()
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| ".".to_string())
    } else {
        watch_path.to_string_lossy().into_owned()
    }
}

// ── build_analysis ────────────────────────────────────────────────────────────

/// Assemble the analysis JSON consumed by `graphify_report::write_report`.
///
/// Mirrors `build_analysis` from `src/main.rs`.
pub(crate) fn build_analysis(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    root: &Path,
) -> Value {
    let mut communities_json = serde_json::Map::new();
    for (cid, members) in communities {
        communities_json.insert(
            cid.to_string(),
            Value::Array(members.iter().map(|m| Value::String(m.clone())).collect()),
        );
    }
    let gods = god_nodes(graph, 12);
    let surprising = surprising_connections(graph, communities, 12);
    let empty_labels: IndexMap<i64, String> = IndexMap::new();
    let suggested = suggest_questions(graph, communities, &empty_labels, 8);
    json!({
        "root": root.display().to_string(),
        "communities": Value::Object(communities_json),
        "god_nodes": gods,
        "surprising_connections": surprising,
        "suggested_questions": suggested,
        "min_community_size": 3,
    })
}

// ── detect_code_files ─────────────────────────────────────────────────────────

/// Run detection and return the code+document files that have AST extractors.
pub(crate) fn detect_code_files(
    watch_path: &Path,
    follow_symlinks: bool,
) -> (DetectResult, Vec<PathBuf>) {
    let follow = if follow_symlinks { Some(true) } else { None };
    let detected = detect(watch_path, follow, None);
    let mut code_files: Vec<PathBuf> = detected
        .files
        .get("code")
        .map(|v| v.iter().map(|s| watch_path.join(s)).collect())
        .unwrap_or_default();

    // Include document files that have AST extractors (e.g. .md, .mdx, .qmd).
    if let Some(docs) = detected.files.get("document") {
        for doc in docs {
            let p = watch_path.join(doc);
            // A file has an extractor if classify_file says it's Code or if
            // the file itself is a markdown-family file we know we can parse.
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
            let has_extractor = matches!(classify_file(&p), Some(FileType::Code))
                || matches!(ext, "md" | "mdx" | "qmd");
            if has_extractor {
                code_files.push(p);
            }
        }
    }

    (detected, code_files)
}

// ── graph_to_topology_value ───────────────────────────────────────────────────

/// Convert a `Graph` to the `node_link_data` dict form used for topology
/// comparison.
///
/// This is a simplified serialisation that only captures the structural shape
/// (nodes + edges) without community assignments — sufficient for the topology
/// comparison in the rebuild pipeline.
pub(crate) fn graph_to_topology_value(graph: &Graph) -> Value {
    let nodes: Vec<Value> = graph
        .nodes()
        .map(|(id, attrs)| {
            let mut m = serde_json::Map::new();
            m.insert("id".to_string(), Value::String(id.clone()));
            for (k, v) in attrs {
                m.insert(k.clone(), v.clone());
            }
            Value::Object(m)
        })
        .collect();

    let edges: Vec<Value> = graph
        .edges()
        .map(|e| {
            let mut m = serde_json::Map::new();
            m.insert("source".to_string(), Value::String(e.source.clone()));
            m.insert("target".to_string(), Value::String(e.target.clone()));
            for (k, v) in &e.attrs {
                m.insert(k.clone(), v.clone());
            }
            Value::Object(m)
        })
        .collect();

    json!({
        "nodes": nodes,
        "links": edges,
        "hyperedges": [],
    })
}
