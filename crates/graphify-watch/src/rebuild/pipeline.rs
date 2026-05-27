//! Inner rebuild pipeline: detect → extract → build → cluster → report →
//! export.
//!
//! Extracted from `rebuild.rs` to separate the sequencing logic from the
//! public entry point (`rebuild_code`) and the smaller helpers.  This module
//! assumes the rebuild lock is already held by the caller.

use std::path::{Path, PathBuf};

use graphify_export::attach_hyperedges;
use serde_json::Value;

use crate::error::WatchError;
use crate::graphify_out;
use crate::rebuild::git::git_head;
use crate::rebuild::helpers::report_root_label;
use crate::rebuild::pipeline_helpers::{
    FinaliseArgs, build_phase, cluster_phase, compare_existing_graph, compare_existing_report,
    compute_extract_targets, detect_phase, extract_phase, finalise_rebuild, load_or_default_labels,
    merge_with_existing_graph, render_report_phase, resolve_project_root, run_analysis,
    run_no_cluster_path, topology_unchanged, write_graph_tmp,
};
use crate::rebuild::relativize::relativize_source_files;

/// Inner rebuild pipeline, called after the lock has been acquired.
///
/// Returns `Ok(true)` when the outputs were updated, `Ok(false)` when
/// no tracked code files were found or no topology changes were detected.
///
/// # Errors
///
/// Propagates I/O and pipeline errors via `WatchError`.
pub(crate) fn rebuild_code_inner(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    force: bool,
    no_cluster: bool,
) -> Result<bool, WatchError> {
    let watch_root = watch_path
        .canonicalize()
        .unwrap_or_else(|_| watch_path.to_path_buf());
    let project_root = resolve_project_root(watch_path, &watch_root);
    let report_root = report_root_label(watch_path);
    let out = watch_path.join(graphify_out());

    let (detected, code_files) = detect_phase(watch_path);
    if code_files.is_empty() {
        println!("[graphify watch] No code files found - nothing to rebuild.");
        return Ok(false);
    }

    let Some(targets) =
        compute_extract_targets(changed_paths, &code_files, &watch_root, &project_root)
    else {
        return Ok(true);
    };
    let extract_targets = targets.wanted;
    let deleted_paths = targets.deleted_paths;

    let commit = git_head(&watch_root);
    let mut result = extract_phase(&extract_targets, &watch_root);
    let t_post = std::time::Instant::now();

    let existing_graph_path = out.join("graph.json");
    let merge = merge_with_existing_graph(
        &mut result,
        &existing_graph_path,
        changed_paths.is_some(),
        &deleted_paths,
        &extract_targets,
        &code_files,
        &project_root,
    );
    let existing_graph_data = merge.existing_graph_data;
    // A full re-extraction that evicts deleted-file nodes is a legitimate
    // shrink, so bypass the guard the same way an explicit deletion does (#1007).
    let had_explicit_deletions = targets.had_tracked_deletion || merge.evicted_deleted_sources;

    relativize_source_files(&mut result, &project_root);
    std::fs::create_dir_all(&out).map_err(WatchError::Io)?;
    std::fs::write(
        out.join(".graphify_root"),
        watch_root.to_string_lossy().as_bytes(),
    )
    .map_err(WatchError::Io)?;

    if no_cluster {
        return run_no_cluster_path(
            &result,
            &existing_graph_path,
            &existing_graph_data,
            &out,
            force,
            had_explicit_deletions,
            t_post,
        );
    }

    let graph = build_phase(&result, watch_path)?;
    if topology_unchanged(&existing_graph_data, &graph, &out) {
        return Ok(true);
    }

    let communities = cluster_phase(&graph, &existing_graph_data);
    let labels_file = out.join(".graphify_labels.json");
    let labels = load_or_default_labels(&labels_file, &communities);

    let analysis = run_analysis(&graph, &communities, watch_path, &report_root);
    let mut graph_with_hyper = graph.clone();
    if let Some(Value::Array(hyper)) = result.get("hyperedges") {
        attach_hyperedges(&mut graph_with_hyper, hyper);
    }

    let graph_tmp = out.join(".graph.tmp.json");
    let Some(candidate_graph_data) = write_graph_tmp(
        &graph_with_hyper,
        &communities,
        &graph_tmp,
        commit.as_deref(),
    )?
    else {
        return Ok(false);
    };

    let same_graph = compare_existing_graph(&existing_graph_path, &candidate_graph_data);
    let report_path = out.join("GRAPH_REPORT.md");
    let report_content = render_report_phase(&graph_with_hyper, &analysis);
    let same_report = compare_existing_report(&report_path, &report_content);
    let no_change = same_graph && same_report;

    finalise_rebuild(&FinaliseArgs {
        no_change,
        force,
        had_explicit_deletions,
        graph_with_hyper: &graph_with_hyper,
        communities: &communities,
        labels: &labels,
        labels_file: &labels_file,
        existing_graph_data: &existing_graph_data,
        candidate_graph_data: &candidate_graph_data,
        graph_tmp: &graph_tmp,
        existing_graph_path: &existing_graph_path,
        report_path: &report_path,
        report_content: &report_content,
        detected: &detected,
        out: &out,
    })?;

    Ok(true)
}
