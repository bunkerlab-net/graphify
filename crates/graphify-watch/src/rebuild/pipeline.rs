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
use crate::rebuild::git::git_head;
use crate::rebuild::helpers::report_root_label;
use crate::rebuild::pipeline_helpers::{
    FinaliseArgs, build_phase, cluster_phase, compare_existing_graph, compare_existing_report,
    compute_extract_targets, compute_rebuilt_sources, detect_phase, extract_phase,
    finalise_rebuild, load_or_default_labels, render_report_phase, resolve_project_root,
    run_analysis, run_no_cluster_path, topology_unchanged, write_graph_tmp,
};
use crate::rebuild::reconcile::{
    filter_semantic_backed_docs, rebase_relative_source_files, reconcile_existing_graph,
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
// Linear detect → extract → merge → build → cluster → finalise pipeline;
// splitting the sequence across more helpers obscures the ordering it encodes.
#[allow(clippy::too_many_lines)]
pub(crate) fn rebuild_code_inner(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    force: bool,
    no_cluster: bool,
    follow_symlinks: bool,
    check_shrink_fn: crate::rebuild::ShrinkChecker,
) -> Result<bool, WatchError> {
    let watch_root = watch_path
        .canonicalize()
        .unwrap_or_else(|_| watch_path.to_path_buf());
    let project_root = resolve_project_root(watch_path, &watch_root);
    let report_root = report_root_label(watch_path);
    let out = watch_path.join(graphify_security::graphify_out());

    // Re-apply the excludes the initial extract persisted, so an update/watch/
    // hook rebuild never silently re-includes deliberately excluded paths (#1886).
    let persisted_excludes = crate::build_config::read_build_excludes(&out);
    let extra_excludes = (!persisted_excludes.is_empty()).then_some(persisted_excludes.as_slice());
    let (detected, code_files) = detect_phase(watch_path, follow_symlinks, extra_excludes);
    let existing_graph_path = out.join("graph.json");
    // Proceed with no code files only when a prior graph exists — the rebuild
    // then reconciles deletions/renames against it (#8d8d2b8). With neither,
    // there is nothing to do.
    if code_files.is_empty() && !existing_graph_path.exists() {
        println!("[graphify watch] No code files found - nothing to rebuild.");
        return Ok(false);
    }

    let Some(targets) =
        compute_extract_targets(changed_paths, &code_files, &watch_root, &project_root)
    else {
        return Ok(true);
    };
    // #1915: never AST-quick-scan a doc already represented by a semantic (LLM)
    // layer in the prior graph — that would mint heading nodes on top of the
    // preserved semantic nodes. The doc stays in `code_files` (corpus + shrink),
    // only its extraction is skipped, so a bloated graph self-heals.
    let extract_targets = filter_semantic_backed_docs(
        targets.wanted,
        &existing_graph_path,
        &out,
        &project_root,
        &watch_root,
    );
    let mut deleted_paths = targets.deleted_paths;
    let deleted_source_identities = targets.deleted_source_identities;

    let commit = git_head(&watch_root);
    let mut result = extract_phase(&extract_targets, &watch_root);
    let t_post = std::time::Instant::now();

    // Rebase cache-root-relative extraction paths onto the project root before
    // reconciliation, so fresh and preserved paths share one root (#8d8d2b8).
    rebase_relative_source_files(&mut result, &watch_root, &project_root);

    let reconcile = reconcile_existing_graph(
        &existing_graph_path,
        &mut result,
        &out,
        &project_root,
        &watch_root,
        &code_files,
        &extract_targets,
        changed_paths.is_none(),
        &mut deleted_paths,
        &deleted_source_identities,
    );
    let existing_graph_data = reconcile.existing_graph_data;

    // Relativise the merged result, scoped to the watched root so a preserved
    // sibling-project node (identity outside the root) is not mis-relativised.
    relativize_source_files(&mut result, &project_root, Some(&watch_root));

    // Re-extracted or deleted sources may legitimately shrink the graph, so the
    // shrink-guard bypass keys off any evicted source (`deleted_paths` now
    // includes reconciliation-discovered removals). Mirrors Python's
    // `had_explicit_deletions=bool(deleted_paths)`.
    // Full rebuild: every in-corpus source is a legitimate shrink basis. A
    // semantic-backed doc excluded from AST extraction stays in `code_files`, so
    // its stale `_origin=="ast"` heading nodes may be shed (self-heal, #1915)
    // while its SEMANTIC nodes are preserved by the origin-aware reconcile
    // (`preserved_nodes`/`preserved_edges`), NOT removed by this shrink basis.
    // Incremental: only the re-extracted targets. Mirrors graphify-py
    // `watch.py:1035-1041` (full → code_files, else → extract_targets) exactly;
    // excluding semantic docs here would diverge and wrongly refuse a legitimate
    // self-heal shrink. (Disputes CodeRabbit's "origin-aware rebuilt basis"
    // finding — origin-awareness lives in the node reconcile, not the basis.)
    let rebuilt_basis: &[PathBuf] = if changed_paths.is_none() {
        &code_files
    } else {
        &extract_targets
    };
    let rebuilt_sources = compute_rebuilt_sources(rebuilt_basis, &deleted_paths, &project_root);
    let had_explicit_deletions = !deleted_paths.is_empty();

    std::fs::create_dir_all(&out).map_err(WatchError::Io)?;

    if no_cluster {
        return run_no_cluster_path(
            &result,
            &existing_graph_path,
            &existing_graph_data,
            &out,
            watch_path,
            force,
            had_explicit_deletions,
            Some(&rebuilt_sources),
            check_shrink_fn,
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
        Some(&labels),
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
        rebuilt_sources: Some(&rebuilt_sources),
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
        project_root: &project_root,
        watch_path,
        check_shrink_fn,
    })?;

    Ok(true)
}
