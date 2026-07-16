//! Small helpers used by the rebuild pipeline.
//!
//! Extracted from `rebuild.rs` to keep the pipeline module focused on
//! sequencing and leave utility functions here.

use std::path::{Path, PathBuf};

use graphify_analyze::{god_nodes, suggest_questions, surprising_connections};
use graphify_build::Graph;
use graphify_detect::{DetectResult, detect};
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

/// Assemble the analysis JSON consumed by `graphify_report::render_report`.
///
/// Emits both the Python-compatible keys (`cohesion`, `gods`, `surprises`,
/// `tokens`) and the Rust report consumer's preferred aliases
/// (`cohesion_scores`, `god_nodes`, `surprising_connections`,
/// `suggested_questions`).  See `src/cli/mod.rs::build_analysis` for the
/// canonical shape — this watch-local copy must stay in sync.
pub(crate) fn build_analysis(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    root: &Path,
    // (input, output) LLM token cost (#1694); `(0, 0)` when no LLM ran.
    token_cost: (u64, u64),
) -> Value {
    let perf = std::env::var("GRAPHIFY_PERF_LOG").is_ok();
    let mut communities_json = serde_json::Map::new();
    for (cid, members) in communities {
        communities_json.insert(
            cid.to_string(),
            Value::Array(members.iter().map(|m| Value::String(m.clone())).collect()),
        );
    }
    let t = std::time::Instant::now();
    let cohesion = graphify_cluster::score_all(graph, communities);
    if perf {
        eprintln!(
            "[perf]   build_analysis/score_all: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let mut cohesion_json = serde_json::Map::new();
    for (cid, score) in &cohesion {
        cohesion_json.insert(
            cid.to_string(),
            serde_json::Number::from_f64(*score).map_or(Value::Null, Value::Number),
        );
    }
    let t = std::time::Instant::now();
    let gods = god_nodes(graph, 12);
    if perf {
        eprintln!(
            "[perf]   build_analysis/god_nodes: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    let surprising = surprising_connections(graph, communities, 12);
    if perf {
        eprintln!(
            "[perf]   build_analysis/surprising: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let empty_labels: IndexMap<i64, String> = IndexMap::new();
    let t = std::time::Instant::now();
    let suggested = suggest_questions(graph, communities, &empty_labels, 8);
    if perf {
        eprintln!(
            "[perf]   build_analysis/suggest_questions: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    json!({
        "root": root.display().to_string(),
        "communities": Value::Object(communities_json),
        "cohesion": Value::Object(cohesion_json.clone()),
        "gods": gods.clone(),
        "surprises": surprising.clone(),
        "tokens": json!({"input": token_cost.0, "output": token_cost.1}),
        "cohesion_scores": Value::Object(cohesion_json),
        "god_nodes": gods,
        "surprising_connections": surprising,
        // `token_cost` is the form `graphify_report::render_report` reads (#1694).
        "token_cost": json!({"input": token_cost.0, "output": token_cost.1}),
        "suggested_questions": suggested,
        "min_community_size": 3,
    })
}

// ── detect_code_files ─────────────────────────────────────────────────────────

/// Run detection and return the code+document files that have AST extractors.
/// `extra_excludes` re-applies persisted `--exclude` patterns (#1886).
pub(crate) fn detect_code_files(
    watch_path: &Path,
    follow_symlinks: bool,
    extra_excludes: Option<&[String]>,
) -> (DetectResult, Vec<PathBuf>) {
    let follow = if follow_symlinks { Some(true) } else { None };
    let detected = detect(watch_path, follow, extra_excludes);
    let mut code_files: Vec<PathBuf> = detected
        .files
        .get("code")
        .map(|v| v.iter().map(|s| watch_path.join(s)).collect())
        .unwrap_or_default();

    // Include document files that have an AST extractor (e.g. `.md`/`.mdx`/`.qmd`,
    // case-insensitively). Mirrors graphify-py's `_get_extractor(p) is not None`
    // filter (watch.py) — the shared predicate keeps the two in lockstep and
    // catches capitalised extensions the old hard-coded list missed.
    if let Some(docs) = detected.files.get("document") {
        for doc in docs {
            let p = watch_path.join(doc);
            if graphify_extract::has_extractor(&p) {
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
