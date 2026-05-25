//! Helper functions split out of [`super::pipeline::rebuild_code_inner`].

use std::path::{Path, PathBuf};

use graphify_build::{Graph, build_from_json};
use graphify_cluster::{cluster, remap_communities_to_previous, score_all};
use graphify_detect::{FileType, classify_file};
use graphify_export::{backup_if_protected, to_html, to_json};
use graphify_extract::extract;
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::canonical::{
    canonical_graph_for_compare, canonical_topology_for_compare, json_text, report_for_compare,
};
use crate::error::WatchError;
use crate::rebuild::community::node_community_map;
use crate::rebuild::helpers::{build_analysis, detect_code_files, graph_to_topology_value};
use crate::rebuild::shrink::check_shrink;

/// Re-export of [`graphify_detect::DetectResult`] used throughout the pipeline.
pub(crate) type DetectResult = graphify_detect::DetectResult;

/// Resolve the project-root path used for source-file relativisation.
pub(crate) fn resolve_project_root(watch_path: &Path, watch_root: &Path) -> PathBuf {
    if watch_path.is_absolute() {
        watch_root.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| watch_root.to_path_buf())
            .canonicalize()
            .unwrap_or_else(|_| watch_root.to_path_buf())
    }
}

/// Run detection and log the elapsed time when `GRAPHIFY_PERF_LOG` is set.
pub(crate) fn detect_phase(watch_path: &Path) -> (DetectResult, Vec<PathBuf>) {
    let t_detect = std::time::Instant::now();
    let (detected, code_files) = detect_code_files(watch_path, false);
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] detect: {:.2}s ({} files)",
            t_detect.elapsed().as_secs_f64(),
            code_files.len()
        );
    }
    (detected, code_files)
}

/// `true` when `path` would have been pulled into the rebuild's `code_files`
/// set if it still existed on disk. Mirrors the inclusion rule used in
/// [`crate::rebuild::helpers::detect_code_files`]: any `FileType::Code` plus
/// the markdown-family documents (`.md` / `.mdx` / `.qmd`) that have AST
/// extractors. Used to narrow the shrink-guard bypass — a deleted
/// `.gitignore` or `.env` is not a tracked-code deletion.
fn is_tracked_code_path(path: &Path) -> bool {
    if matches!(classify_file(path), Some(FileType::Code)) {
        return true;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    matches!(ext, "md" | "mdx" | "qmd")
}

/// Result of [`compute_extract_targets`]: which files to extract from, which
/// paths to evict from the existing graph, and whether the change set declared
/// a tracked-code-file deletion (relevant for the shrink-guard bypass).
pub(crate) struct ExtractTargets {
    /// Existing tracked code files that the caller wants re-extracted.
    pub wanted: Vec<PathBuf>,
    /// Paths to evict from any prior graph — covers both true deletions and
    /// non-code paths in the change set whose nodes should drop out.
    pub deleted_paths: Vec<String>,
    /// `true` when the change set contained at least one path that no longer
    /// exists on disk. The watch shrink-guard bypass is keyed off this flag
    /// (not `!deleted_paths.is_empty()`) so a changed-but-untracked README
    /// can't accidentally suppress the guard.
    pub had_tracked_deletion: bool,
}

/// Compute the list of files to extract from + the list of deleted paths to evict.
///
/// Returns `None` when there's nothing to do (no tracked files in the change set).
pub(crate) fn compute_extract_targets(
    changed_paths: Option<&[PathBuf]>,
    code_files: &[PathBuf],
    watch_root: &Path,
    project_root: &Path,
) -> Option<ExtractTargets> {
    let Some(changed) = changed_paths else {
        return Some(ExtractTargets {
            wanted: code_files.to_vec(),
            deleted_paths: Vec::new(),
            had_tracked_deletion: false,
        });
    };
    let code_set: std::collections::HashSet<PathBuf> = code_files
        .iter()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
        .collect();
    let mut wanted: Vec<PathBuf> = Vec::new();
    let mut deleted_paths: Vec<String> = Vec::new();
    let mut had_tracked_deletion = false;
    for raw in changed {
        let cand = if raw.is_absolute() {
            raw.canonicalize().unwrap_or_else(|_| raw.clone())
        } else {
            (watch_root.join(raw))
                .canonicalize()
                .unwrap_or_else(|_| watch_root.join(raw))
        };
        if cand.exists() && code_set.contains(&cand) {
            wanted.push(cand);
        } else {
            // A path that doesn't exist on disk is a deletion. Only flip the
            // shrink-guard bypass when the deleted path's extension matches
            // the inclusion rule used by `detect_code_files` (code files plus
            // markdown-family documents that have AST extractors). A deleted
            // `.gitignore` or `.env` is not a tracked-code deletion and must
            // not suppress the guard. A path that exists but isn't in
            // `code_set` (e.g. a touched README that lives outside the watch
            // root) still earns eviction so its stale nodes drop out.
            if !cand.exists() && is_tracked_code_path(&cand) {
                had_tracked_deletion = true;
            }
            let rel = cand.strip_prefix(project_root).map_or_else(
                |_| cand.to_string_lossy().into_owned(),
                |p| p.to_string_lossy().into_owned(),
            );
            deleted_paths.push(rel);
        }
    }
    if wanted.is_empty() && deleted_paths.is_empty() {
        println!("[graphify watch] No tracked code files in change set - skipping rebuild.");
        return None;
    }
    Some(ExtractTargets {
        wanted,
        deleted_paths,
        had_tracked_deletion,
    })
}

/// Run AST extraction on the given targets, returning the canonical result JSON.
pub(crate) fn extract_phase(extract_targets: &[PathBuf], watch_root: &Path) -> Value {
    let t_extract = std::time::Instant::now();
    let result = if extract_targets.is_empty() {
        json!({
            "nodes": [],
            "edges": [],
            "hyperedges": [],
            "input_tokens": 0,
            "output_tokens": 0,
        })
    } else {
        let output = extract(extract_targets, Some(watch_root));
        json!({
            "nodes": output.nodes,
            "edges": output.edges,
            "hyperedges": [],
            "input_tokens": output.input_tokens,
            "output_tokens": output.output_tokens,
        })
    };
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] extract: {:.2}s ({} targets)",
            t_extract.elapsed().as_secs_f64(),
            extract_targets.len()
        );
    }
    result
}

/// Merge AST-extracted nodes/edges with preserved entries from any prior `graph.json`.
///
/// Returns the existing graph JSON (or `Value::Null` when absent) so the caller can
/// reuse it for downstream comparisons.
#[allow(clippy::too_many_lines)] // single-pass merge — splitting would obscure ordering
pub(crate) fn merge_with_existing_graph(
    result: &mut Value,
    existing_graph_path: &Path,
    has_changed_paths: bool,
    deleted_paths: &[String],
    extract_targets: &[PathBuf],
    project_root: &Path,
) -> Value {
    // Reject oversized graph files before reading them into memory — mirrors
    // the size-cap guard added in `graphify-py/graphify/watch.py`. Surface
    // the rejection on stderr so the user knows we skipped the merge.
    if let Err(err) = graphify_security::check_graph_file_size_cap(existing_graph_path) {
        eprintln!(
            "[graphify watch] skipping merge with existing graph at {}: {err}",
            existing_graph_path.display()
        );
        return Value::Null;
    }
    let Ok(text) = std::fs::read_to_string(existing_graph_path) else {
        return Value::Null;
    };
    let Ok(existing) = serde_json::from_str::<Value>(&text) else {
        return Value::Null;
    };
    let existing_graph_data = existing.clone();

    let new_ast_ids: std::collections::HashSet<String> = result
        .get("nodes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut evict_sources: std::collections::HashSet<String> =
        deleted_paths.iter().cloned().collect();
    if has_changed_paths {
        for p in extract_targets {
            let rel = p.strip_prefix(project_root).map_or_else(
                |_| p.to_string_lossy().into_owned(),
                |r| r.to_string_lossy().into_owned(),
            );
            evict_sources.insert(rel);
        }
    }

    let preserved_nodes: Vec<Value> = existing
        .get("nodes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|n| {
                    let id = n.get("id").and_then(Value::as_str).unwrap_or("");
                    if new_ast_ids.contains(id) {
                        return false;
                    }
                    if !evict_sources.is_empty() {
                        let src = n.get("source_file").and_then(Value::as_str).unwrap_or("");
                        if evict_sources.contains(src) {
                            return false;
                        }
                    }
                    true
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let all_ids: std::collections::HashSet<String> = new_ast_ids
        .iter()
        .cloned()
        .chain(
            preserved_nodes
                .iter()
                .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string)),
        )
        .collect();

    let preserved_edges: Vec<Value> = existing
        .get("links")
        .or_else(|| existing.get("edges"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|e| {
                    let src = e.get("source").and_then(Value::as_str).unwrap_or("");
                    let tgt = e.get("target").and_then(Value::as_str).unwrap_or("");
                    all_ids.contains(src) && all_ids.contains(tgt)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let mut merged_nodes: Vec<Value> = result
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    merged_nodes.extend(preserved_nodes);

    let mut merged_edges: Vec<Value> = result
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    merged_edges.extend(preserved_edges);

    let hyper = existing
        .get("hyperedges")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));

    *result = json!({
        "nodes": merged_nodes,
        "edges": merged_edges,
        "hyperedges": hyper,
        "input_tokens": 0,
        "output_tokens": 0,
    });
    existing_graph_data
}

/// Execute the `--no-cluster` shortcut: write `graph.json` only, no clustering or report.
pub(crate) fn run_no_cluster_path(
    result: &Value,
    existing_graph_path: &Path,
    existing_graph_data: &Value,
    out: &Path,
    force: bool,
    had_explicit_deletions: bool,
    t_post: std::time::Instant,
) -> Result<bool, WatchError> {
    let edges = result
        .get("edges")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    let mut candidate: serde_json::Map<String, Value> =
        result.as_object().cloned().unwrap_or_default();
    candidate.remove("edges");
    candidate.insert("links".to_string(), edges);
    let candidate_data = Value::Object(candidate);

    let same_graph = compare_existing_graph(existing_graph_path, &candidate_data);

    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] post-extract canonicalize+compare: {:.2}s",
            t_post.elapsed().as_secs_f64()
        );
    }
    let t_write = std::time::Instant::now();
    if !same_graph {
        check_shrink(
            force,
            existing_graph_data,
            &candidate_data,
            None,
            had_explicit_deletions,
        )?;
        std::fs::write(existing_graph_path, json_text(&candidate_data).as_bytes())
            .map_err(WatchError::Io)?;
    }
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] write graph.json: {:.2}s",
            t_write.elapsed().as_secs_f64()
        );
    }

    let flag = out.join("needs_update");
    if flag.exists() {
        let _ = std::fs::remove_file(&flag);
    }

    if same_graph {
        println!(
            "[graphify watch] No code-graph changes detected (--no-cluster); \
             outputs left untouched."
        );
    } else {
        let n_nodes = candidate_data
            .get("nodes")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let n_edges = candidate_data
            .get("links")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        println!("[graphify watch] Rebuilt (no clustering): {n_nodes} nodes, {n_edges} edges");
        println!("[graphify watch] graph.json updated in {}", out.display());
    }
    Ok(true)
}

/// Build the in-memory `Graph` from the merged AST/extraction JSON.
pub(crate) fn build_phase(result: &Value, watch_path: &Path) -> Result<Graph, WatchError> {
    let t_build = std::time::Instant::now();
    let graph = build_from_json(result.clone(), true, Some(watch_path))
        .map_err(|e| WatchError::Pipeline(e.to_string()))?;
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] build_from_json: {:.2}s ({} nodes, {} edges)",
            t_build.elapsed().as_secs_f64(),
            graph.node_count(),
            graph.edge_count()
        );
    }
    Ok(graph)
}

/// Return `true` and prune `needs_update` when the rebuilt graph topology matches `existing`.
pub(crate) fn topology_unchanged(existing_graph_data: &Value, graph: &Graph, out: &Path) -> bool {
    if existing_graph_data.is_null() {
        return false;
    }
    let candidate_topology = graph_to_topology_value(graph);
    let a = serde_json::to_string(&canonical_topology_for_compare(existing_graph_data))
        .unwrap_or_default();
    let b = serde_json::to_string(&canonical_topology_for_compare(&candidate_topology))
        .unwrap_or_default();
    if a != b {
        return false;
    }
    let flag = out.join("needs_update");
    if flag.exists() {
        let _ = std::fs::remove_file(&flag);
    }
    println!(
        "[graphify watch] No code-graph topology changes detected; \
         outputs left untouched."
    );
    true
}

/// Run Louvain clustering (remapping to previous community IDs when available) +
/// log cohesion-scoring time.
pub(crate) fn cluster_phase(
    graph: &Graph,
    existing_graph_data: &Value,
) -> IndexMap<i64, Vec<String>> {
    let t_cluster = std::time::Instant::now();
    let previous_community_map = node_community_map(existing_graph_data);
    let mut communities = cluster(graph, 1.0, None);
    if !previous_community_map.is_empty() {
        communities = remap_communities_to_previous(&communities, &previous_community_map);
    }
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] cluster: {:.2}s ({} communities)",
            t_cluster.elapsed().as_secs_f64(),
            communities.len()
        );
    }
    let t_cohesion = std::time::Instant::now();
    let _cohesion = score_all(graph, &communities);
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] score_all: {:.2}s",
            t_cohesion.elapsed().as_secs_f64()
        );
    }
    communities
}

/// Load the persistent labels file, top-up missing entries with the default `"Community <cid>"`.
pub(crate) fn load_or_default_labels(
    labels_file: &Path,
    communities: &IndexMap<i64, Vec<String>>,
) -> IndexMap<i64, String> {
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    if labels_file.exists()
        && let Ok(text) = std::fs::read_to_string(labels_file)
        && let Ok(raw) = serde_json::from_str::<Value>(&text)
        && let Some(obj) = raw.as_object()
    {
        for (k, v) in obj {
            if let (Ok(cid), Some(label)) = (k.parse::<i64>(), v.as_str())
                && communities.contains_key(&cid)
            {
                labels.insert(cid, label.to_string());
            }
        }
    }
    for cid in communities.keys() {
        labels
            .entry(*cid)
            .or_insert_with(|| format!("Community {cid}"));
    }
    labels
}

/// Run analysis (god nodes, surprises, etc.) and override the `root` label.
pub(crate) fn run_analysis(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    watch_path: &Path,
    report_root: &str,
) -> Value {
    let t_analysis = std::time::Instant::now();
    let mut analysis = build_analysis(graph, communities, watch_path);
    if let Some(obj) = analysis.as_object_mut() {
        obj.insert("root".to_string(), Value::String(report_root.to_string()));
    }
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] build_analysis: {:.2}s",
            t_analysis.elapsed().as_secs_f64()
        );
    }
    analysis
}

/// Write the candidate graph JSON to `graph_tmp`. Returns `None` if [`to_json`]
/// refused the write (e.g. shrink guard rejected silent shrinkage).
pub(crate) fn write_graph_tmp(
    graph_with_hyper: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    graph_tmp: &Path,
    commit: Option<&str>,
) -> Result<Option<Value>, WatchError> {
    let t_to_json = std::time::Instant::now();
    let json_written = to_json(graph_with_hyper, communities, graph_tmp, true, commit)
        .map_err(|e| WatchError::Pipeline(e.to_string()))?;
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!("[perf] to_json: {:.2}s", t_to_json.elapsed().as_secs_f64());
    }
    if !json_written {
        return Ok(None);
    }
    if !graph_tmp.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(graph_tmp).map_err(WatchError::Io)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| WatchError::Pipeline(e.to_string()))?;
    Ok(Some(v))
}

/// Returns `true` when the candidate graph data matches what's already on disk.
pub(crate) fn compare_existing_graph(existing_graph_path: &Path, candidate_data: &Value) -> bool {
    if let Err(err) = graphify_security::check_graph_file_size_cap(existing_graph_path) {
        eprintln!(
            "[graphify watch] skipping graph comparison at {}: {err}",
            existing_graph_path.display()
        );
        return false;
    }
    let Ok(text) = std::fs::read_to_string(existing_graph_path) else {
        return false;
    };
    let Ok(existing_payload) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let a =
        serde_json::to_string(&canonical_graph_for_compare(&existing_payload)).unwrap_or_default();
    let b = serde_json::to_string(&canonical_graph_for_compare(candidate_data)).unwrap_or_default();
    a == b
}

/// Render the report and log the elapsed time when `GRAPHIFY_PERF_LOG` is set.
pub(crate) fn render_report_phase(graph_with_hyper: &Graph, analysis: &Value) -> String {
    let t_report = std::time::Instant::now();
    let report_content = graphify_report::render_report(graph_with_hyper, analysis);
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] render_report: {:.2}s",
            t_report.elapsed().as_secs_f64()
        );
    }
    report_content
}

/// Compare the rendered report to the existing on-disk version, ignoring metadata-only changes.
pub(crate) fn compare_existing_report(report_path: &Path, report_content: &str) -> bool {
    let Ok(old) = std::fs::read_to_string(report_path) else {
        return false;
    };
    report_for_compare(&old) == report_for_compare(report_content)
}

/// Arguments for [`commit_rebuild_outputs`] — bundles the shrink guard + atomic
/// rename + sidecar writes into one call.
#[allow(clippy::struct_field_names)]
pub(crate) struct CommitArgs<'a> {
    /// Bypass the shrink guard when `true`.
    pub force: bool,
    /// Skip the shrink guard when the caller has declared deletions — the
    /// smaller graph is expected and not a sign of silent corruption.
    pub had_explicit_deletions: bool,
    /// The graph JSON that was on disk before this rebuild began.
    pub existing_graph_data: &'a Value,
    /// The newly built graph JSON to be committed.
    pub candidate_graph_data: &'a Value,
    /// Temporary file path where the candidate graph was written.
    pub graph_tmp: &'a Path,
    /// Destination path for the committed `graph.json`.
    pub existing_graph_path: &'a Path,
    /// Path to the `GRAPH_REPORT.md` file.
    pub report_path: &'a Path,
    /// Rendered report content to write to `report_path`.
    pub report_content: &'a str,
    /// Community ID → human-readable label mapping.
    pub labels: &'a IndexMap<i64, String>,
    /// Persistent labels JSON file path.
    pub labels_file: &'a Path,
    /// Detection result used to produce the AST manifest.
    pub detected: &'a DetectResult,
    /// Output directory where all artefacts are written.
    pub out: &'a Path,
}

/// Atomically commit the rebuild outputs: `graph.json`, `GRAPH_REPORT.md`,
/// labels, and the AST manifest.
pub(crate) fn commit_rebuild_outputs(args: &CommitArgs<'_>) -> Result<(), WatchError> {
    check_shrink(
        args.force,
        args.existing_graph_data,
        args.candidate_graph_data,
        Some(args.graph_tmp),
        args.had_explicit_deletions,
    )?;
    let _ = backup_if_protected(args.out);
    std::fs::rename(args.graph_tmp, args.existing_graph_path).map_err(WatchError::Io)?;
    std::fs::write(args.report_path, args.report_content).map_err(WatchError::Io)?;

    let labels_json_val: serde_json::Map<String, Value> = args
        .labels
        .iter()
        .map(|(k, v)| (k.to_string(), Value::String(v.clone())))
        .collect();
    std::fs::write(
        args.labels_file,
        json_text(&Value::Object(labels_json_val)).as_bytes(),
    )
    .map_err(WatchError::Io)?;

    let manifest_path = args.out.join("manifest.json");
    let files_indexed: IndexMap<String, Vec<String>> = args
        .detected
        .files
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Err(e) = graphify_detect::save_manifest(&files_indexed, &manifest_path, "ast") {
        println!("[graphify watch] warning: could not write manifest: {e}");
    }
    Ok(())
}

/// Render the interactive HTML viz, returning `true` on success.
pub(crate) fn render_html_phase(
    graph_with_hyper: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    labels: &IndexMap<i64, String>,
    out: &Path,
) -> bool {
    let labels_for_html: IndexMap<i64, String> = labels.clone();
    let html_path = out.join("graph.html");
    let t_html = std::time::Instant::now();
    let html_result = to_html(
        graph_with_hyper,
        communities,
        &html_path,
        Some(&labels_for_html),
        None,
        None,
    );
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!("[perf] to_html: {:.2}s", t_html.elapsed().as_secs_f64());
    }
    match html_result {
        Ok(()) => true,
        Err(e) => {
            println!("[graphify watch] Skipped graph.html: {e}");
            if html_path.exists() {
                let _ = std::fs::remove_file(&html_path);
            }
            false
        }
    }
}

/// Arguments for [`finalise_rebuild`] — commits outputs, prunes flags, renders
/// HTML, and prints the final summary.
#[allow(clippy::struct_field_names)]
pub(crate) struct FinaliseArgs<'a> {
    /// When `true`, both the graph and report are unchanged; outputs are left untouched.
    pub no_change: bool,
    /// Bypass the shrink guard when `true`.
    pub force: bool,
    /// Skip the shrink guard when the caller has declared deletions — see
    /// [`check_shrink`](crate::rebuild::shrink::check_shrink) for context.
    pub had_explicit_deletions: bool,
    /// Final graph value including attached hyperedges.
    pub graph_with_hyper: &'a Graph,
    /// Community detection result mapping community ID → member node IDs.
    pub communities: &'a IndexMap<i64, Vec<String>>,
    /// Community ID → human-readable label mapping.
    pub labels: &'a IndexMap<i64, String>,
    /// Persistent labels JSON file path.
    pub labels_file: &'a std::path::Path,
    /// The graph JSON that was on disk before this rebuild began.
    pub existing_graph_data: &'a Value,
    /// The newly built graph JSON to be committed.
    pub candidate_graph_data: &'a Value,
    /// Temporary file path where the candidate graph was written.
    pub graph_tmp: &'a std::path::Path,
    /// Destination path for the committed `graph.json`.
    pub existing_graph_path: &'a std::path::Path,
    /// Path to the `GRAPH_REPORT.md` file.
    pub report_path: &'a std::path::Path,
    /// Rendered report content to write to `report_path`.
    pub report_content: &'a str,
    /// Detection result used to produce the AST manifest.
    pub detected: &'a DetectResult,
    /// Output directory where all artefacts are written.
    pub out: &'a std::path::Path,
}

/// Finalise the rebuild: commit (or skip), prune `needs_update`, render the HTML
/// viz, and print the summary line.
pub(crate) fn finalise_rebuild(args: &FinaliseArgs<'_>) -> Result<(), WatchError> {
    if args.no_change {
        let _ = std::fs::remove_file(args.graph_tmp);
        println!(
            "[graphify watch] No code-graph changes detected; \
             graph.json/GRAPH_REPORT.md left untouched."
        );
    } else {
        commit_rebuild_outputs(&CommitArgs {
            force: args.force,
            had_explicit_deletions: args.had_explicit_deletions,
            existing_graph_data: args.existing_graph_data,
            candidate_graph_data: args.candidate_graph_data,
            graph_tmp: args.graph_tmp,
            existing_graph_path: args.existing_graph_path,
            report_path: args.report_path,
            report_content: args.report_content,
            labels: args.labels,
            labels_file: args.labels_file,
            detected: args.detected,
            out: args.out,
        })?;
    }
    let flag = args.out.join("needs_update");
    if flag.exists() {
        let _ = std::fs::remove_file(&flag);
    }
    let html_written = if args.no_change {
        false
    } else {
        render_html_phase(
            args.graph_with_hyper,
            args.communities,
            args.labels,
            args.out,
        )
    };
    if !args.no_change {
        print_rebuild_summary(
            args.graph_with_hyper,
            args.communities.len(),
            html_written,
            args.out,
        );
    }
    Ok(())
}

/// Print the final "Rebuilt: X nodes, Y edges, Z communities" summary.
pub(crate) fn print_rebuild_summary(
    graph_with_hyper: &Graph,
    n_communities: usize,
    html_written: bool,
    out: &Path,
) {
    println!(
        "[graphify watch] Rebuilt: {} nodes, {} edges, {} communities",
        graph_with_hyper.node_count(),
        graph_with_hyper.edge_count(),
        n_communities
    );
    let mut products = String::from("graph.json");
    if html_written {
        products.push_str(", graph.html");
    }
    products.push_str(" and GRAPH_REPORT.md");
    println!("[graphify watch] {products} updated in {}", out.display());
}
