//! Helper functions split out of [`super::pipeline::rebuild_code_inner`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_build::{Graph, build_from_json, dedupe_edges, dedupe_nodes, norm_source_file};
use graphify_cluster::{cluster, remap_communities_to_previous, score_all};
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
use crate::rebuild::reconcile::lexical_abs;

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
/// `extra_excludes` re-applies persisted `--exclude` patterns (#1886).
pub(crate) fn detect_phase(
    watch_path: &Path,
    follow_symlinks: bool,
    extra_excludes: Option<&[String]>,
) -> (DetectResult, Vec<PathBuf>) {
    let t_detect = std::time::Instant::now();
    let (detected, code_files) = detect_code_files(watch_path, follow_symlinks, extra_excludes);
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] detect: {:.2}s ({} files)",
            t_detect.elapsed().as_secs_f64(),
            code_files.len()
        );
    }
    (detected, code_files)
}

/// The set of source files re-extracted this run, normalised to match the
/// stored graph's `source_file` values. A net shrink is legitimate when every
/// lost node belongs to one of these (or a deleted file) — see
/// [`check_shrink`](crate::rebuild::shrink::check_shrink) (#1116).
pub(crate) fn compute_rebuilt_sources(
    extract_targets: &[PathBuf],
    deleted_paths: &[String],
    project_root: &Path,
) -> HashSet<String> {
    let root = project_root.to_string_lossy();
    let mut sources: HashSet<String> = extract_targets
        .iter()
        .map(|p| {
            let raw = p.to_string_lossy();
            let normalized = norm_source_file(&raw, Some(&root));
            if normalized.is_empty() {
                raw.into_owned()
            } else {
                normalized
            }
        })
        .collect();
    sources.extend(deleted_paths.iter().cloned());
    sources
}

/// Plausible absolute locations for a hook-provided changed path.
///
/// Git hooks pass paths relative to the repository root, but watch callers may
/// pass them relative to the watched root. Keep both interpretations so a graph
/// rooted at `src` accepts both `src/app.py` and `app.py`. Each interpretation
/// yields BOTH a lexical absolute (Python `os.path.abspath` — usable for a
/// deleted file) and the symlink-resolved form, deduped. Mirrors
fn changed_path_candidates(raw: &Path, change_root: &Path, watch_root: &Path) -> Vec<PathBuf> {
    // Both a lexical absolute (Python `os.path.abspath`, `.`/`..` collapsed, no
    // symlink resolution — usable for a deleted file) AND the symlink-resolved
    // form, deduped. Mirrors `_changed_path_candidates` (#8d8d2b8).
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let push = |cand: PathBuf, candidates: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>| {
        if seen.insert(cand.clone()) {
            candidates.push(cand);
        }
    };
    if raw.is_absolute() {
        let lexical = PathBuf::from(lexical_abs(raw));
        let resolved = raw.canonicalize().unwrap_or_else(|_| lexical.clone());
        push(lexical, &mut candidates, &mut seen);
        push(resolved, &mut candidates, &mut seen);
        return candidates;
    }
    for base in [change_root, watch_root] {
        let lexical = PathBuf::from(lexical_abs(&base.join(raw)));
        let resolved = lexical.canonicalize().unwrap_or_else(|_| lexical.clone());
        push(lexical, &mut candidates, &mut seen);
        push(resolved, &mut candidates, &mut seen);
    }
    candidates
}

/// Record `path` as an evicted source: its absolute lexical identity (for the
/// reconciliation's identity-based eviction) plus its `source_file` form under
/// BOTH the project and watched roots (so eviction matches however the prior
/// graph relativised paths). Mirrors `_add_deleted_source` (#8d8d2b8).
fn add_deleted_source(
    deleted_paths: &mut Vec<String>,
    deleted_source_identities: &mut HashSet<String>,
    path: &Path,
    project_root: &Path,
    watch_root: &Path,
) {
    deleted_source_identities.insert(lexical_abs(path));
    for root in [project_root, watch_root] {
        let rel = norm_source_file(&path.to_string_lossy(), Some(&root.to_string_lossy()));
        let rel = if rel.is_empty() {
            path.to_string_lossy().into_owned()
        } else {
            rel
        };
        if !deleted_paths.contains(&rel) {
            deleted_paths.push(rel);
        }
    }
}

/// Result of [`compute_extract_targets`]: which files to extract from, the
/// `source_file` forms to evict, and their absolute identities.
pub(crate) struct ExtractTargets {
    /// Existing tracked code files that the caller wants re-extracted.
    pub wanted: Vec<PathBuf>,
    /// `source_file` forms to evict from any prior graph — deletions plus
    /// non-code paths in the change set whose nodes should drop out.
    pub deleted_paths: Vec<String>,
    /// Absolute lexical identities of the evicted sources (identity-based
    /// eviction in the reconciliation, robust to root/rename differences).
    pub deleted_source_identities: HashSet<String>,
}

/// Compute the files to extract from + the paths/identities to evict.
///
/// Returns `None` when there's nothing to do (no tracked files, no deletions).
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
            deleted_source_identities: HashSet::new(),
        });
    };
    // Git hooks emit repo-root-relative paths; resolve candidates against the
    // current working directory (the change root) as well as the watched root.
    let change_root = std::env::current_dir()
        .and_then(|cwd| cwd.canonicalize())
        .unwrap_or_else(|_| watch_root.to_path_buf());
    // Lexical absolute (Python `os.path.abspath`), matching the candidate forms.
    let code_set: HashSet<PathBuf> = code_files
        .iter()
        .map(|p| PathBuf::from(lexical_abs(p)))
        .collect();
    let mut wanted: Vec<PathBuf> = Vec::new();
    let mut deleted_paths: Vec<String> = Vec::new();
    let mut deleted_source_identities: HashSet<String> = HashSet::new();
    for raw in changed {
        let candidates = changed_path_candidates(raw, &change_root, watch_root);

        // A candidate that still exists and is a tracked code file: re-extract it.
        if let Some(tracked) = candidates
            .iter()
            .find(|cand| cand.exists() && code_set.contains(cand.as_path()))
            .cloned()
        {
            if !wanted.contains(&tracked) {
                wanted.push(tracked);
            }
            continue;
        }

        // Exists under the watched root but detect filtered it out (vendored /
        // gitignored / non-code): evict any stale nodes still claiming it.
        if let Some(existing) = candidates
            .iter()
            .find(|cand| cand.exists() && cand.starts_with(watch_root))
        {
            add_deleted_source(
                &mut deleted_paths,
                &mut deleted_source_identities,
                existing,
                project_root,
                watch_root,
            );
            continue;
        }

        // Deleted or renamed away inside the watched root: evict its nodes.
        if let Some(deleted) = candidates.iter().find(|cand| cand.starts_with(watch_root)) {
            add_deleted_source(
                &mut deleted_paths,
                &mut deleted_source_identities,
                deleted,
                project_root,
                watch_root,
            );
        }
    }
    if wanted.is_empty() && deleted_paths.is_empty() {
        println!("[graphify watch] No tracked code files in change set - skipping rebuild.");
        return None;
    }
    Some(ExtractTargets {
        wanted,
        deleted_paths,
        deleted_source_identities,
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

/// Execute the `--no-cluster` shortcut: write `graph.json` only, no clustering or report.
// One-shot `--no-cluster` shortcut threading the rebuild context; an args
// struct would add indirection for a single call site.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_no_cluster_path(
    result: &Value,
    existing_graph_path: &Path,
    existing_graph_data: &Value,
    out: &Path,
    watch_path: &Path,
    force: bool,
    had_explicit_deletions: bool,
    rebuilt_sources: Option<&HashSet<String>>,
    check_shrink_fn: crate::rebuild::ShrinkChecker,
    t_post: std::time::Instant,
) -> Result<bool, WatchError> {
    // Dedupe nodes by id and parallel edges by (source, target, relation): the
    // clustered path's DiGraph collapses these implicitly, but --no-cluster +
    // repeated `update` concatenate edge lists raw and accumulate duplicates,
    // so edge counts diverge across build modes without this (#1317).
    let deduped_nodes = dedupe_nodes(
        result
            .get("nodes")
            .and_then(Value::as_array)
            .map_or(&[][..], |v| v.as_slice()),
    );
    let deduped_edges = dedupe_edges(
        result
            .get("edges")
            .and_then(Value::as_array)
            .map_or(&[][..], |v| v.as_slice()),
    );
    let mut candidate: serde_json::Map<String, Value> =
        result.as_object().cloned().unwrap_or_default();
    candidate.remove("edges");
    candidate.remove("nodes");
    candidate.insert("nodes".to_string(), Value::Array(deduped_nodes));
    candidate.insert("links".to_string(), Value::Array(deduped_edges));
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
        check_shrink_fn(
            force,
            existing_graph_data,
            &candidate_data,
            None,
            had_explicit_deletions,
            rebuilt_sources,
        )?;
        std::fs::write(existing_graph_path, json_text(&candidate_data).as_bytes())
            .map_err(WatchError::Io)?;
    }
    // #8d8d2b8: write the user-supplied `.graphify_root` marker only after the
    // candidate graph is accepted (or unchanged), so a refused shrink — which
    // returns early above via `check_shrink`'s `?` — can never leave the marker
    // describing a new root while graph.json still holds paths under the old one.
    std::fs::write(
        out.join(".graphify_root"),
        watch_path.to_string_lossy().as_bytes(),
    )
    .map_err(WatchError::Io)?;
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
    let mut analysis = build_analysis(graph, communities, watch_path, (0, 0));
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
    community_labels: Option<&IndexMap<i64, String>>,
) -> Result<Option<Value>, WatchError> {
    let t_to_json = std::time::Instant::now();
    // Forward community labels so `update`/hook rebuilds write readable
    // `community_name` fields, matching the `cluster-only` path (#1808).
    let json_written = to_json(
        graph_with_hyper,
        communities,
        graph_tmp,
        true,
        commit,
        community_labels,
    )
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
    let report_content = graphify_report::render_report(graph_with_hyper, analysis, false);
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
    /// Source files re-extracted this run; lets the shrink guard allow a
    /// symbol removed from a rebuilt file (#1116).
    pub rebuilt_sources: Option<&'a HashSet<String>>,
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
    /// Project root used to relativise manifest keys (#777).
    pub project_root: &'a Path,
    /// Shrink-guard to apply before committing (injectable for tests).
    pub check_shrink_fn: crate::rebuild::ShrinkChecker,
}

/// Atomically commit the rebuild outputs: `graph.json`, `GRAPH_REPORT.md`,
/// labels, and the AST manifest.
pub(crate) fn commit_rebuild_outputs(args: &CommitArgs<'_>) -> Result<(), WatchError> {
    (args.check_shrink_fn)(
        args.force,
        args.existing_graph_data,
        args.candidate_graph_data,
        Some(args.graph_tmp),
        args.had_explicit_deletions,
        args.rebuilt_sources,
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
    // Relativise manifest keys against the project root so a committed
    // graphify-out/ ports across clones (#777). This is a full-scan save, so
    // pass the complete corpus as scan_corpus: rows for files that left the scan
    // but still exist on disk (newly excluded) are pruned instead of surviving
    // as phantom "deleted" entries (#1908).
    let scan_corpus: Vec<String> = files_indexed.values().flatten().cloned().collect();
    if let Err(e) = graphify_detect::save_manifest_to_path_with_root(
        &files_indexed,
        &manifest_path,
        "ast",
        Some(args.project_root),
        Some(&scan_corpus),
    ) {
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
    /// Source files re-extracted this run; lets the shrink guard allow a
    /// symbol removed from a rebuilt file (#1116).
    pub rebuilt_sources: Option<&'a HashSet<String>>,
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
    /// Project root used to relativise manifest keys (#777).
    pub project_root: &'a std::path::Path,
    /// User-supplied watch path, written to `.graphify_root` after acceptance.
    pub watch_path: &'a std::path::Path,
    /// Shrink-guard to apply before committing (injectable for tests).
    pub check_shrink_fn: crate::rebuild::ShrinkChecker,
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
            rebuilt_sources: args.rebuilt_sources,
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
            project_root: args.project_root,
            check_shrink_fn: args.check_shrink_fn,
        })?;
    }
    // #8d8d2b8: persist the `.graphify_root` marker only after the graph is
    // committed (or found unchanged) — a refused shrink returns via the `?`
    // above, so the marker never describes a root that graph.json doesn't match.
    std::fs::write(
        args.out.join(".graphify_root"),
        args.watch_path.to_string_lossy().as_bytes(),
    )
    .map_err(WatchError::Io)?;
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
