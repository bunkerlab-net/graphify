//! Inner rebuild pipeline: detect → extract → build → cluster → report →
//! export.
//!
//! Extracted from `rebuild.rs` to separate the sequencing logic from the
//! public entry point (`rebuild_code`) and the smaller helpers.  This module
//! assumes the rebuild lock is already held by the caller.

use std::path::{Path, PathBuf};

use graphify_build::build_from_json;
use graphify_cluster::{cluster, remap_communities_to_previous, score_all};
use graphify_export::{attach_hyperedges, backup_if_protected, to_html, to_json};
use graphify_extract::extract;
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::canonical::{
    canonical_graph_for_compare, canonical_topology_for_compare, json_text, report_for_compare,
};
use crate::error::WatchError;
use crate::graphify_out;
use crate::rebuild::community::node_community_map;
use crate::rebuild::git::git_head;
use crate::rebuild::helpers::{
    build_analysis, detect_code_files, graph_to_topology_value, report_root_label,
};
use crate::rebuild::relativize::relativize_source_files;
use crate::rebuild::shrink::check_shrink;

/// Inner rebuild pipeline, called after the lock has been acquired.
///
/// Returns `Ok(true)` when the outputs were updated, `Ok(false)` when
/// no tracked code files were found or no topology changes were detected.
///
/// # Errors
///
/// Propagates I/O and pipeline errors via `WatchError`.
#[allow(clippy::too_many_lines)] // mirrors Python's _rebuild_code which is ~300 lines
pub(crate) fn rebuild_code_inner(
    watch_path: &Path,
    changed_paths: Option<&[PathBuf]>,
    force: bool,
    no_cluster: bool,
) -> Result<bool, WatchError> {
    let watch_root = watch_path
        .canonicalize()
        .unwrap_or_else(|_| watch_path.to_path_buf());
    let project_root = if watch_path.is_absolute() {
        watch_root.clone()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| watch_root.clone())
            .canonicalize()
            .unwrap_or_else(|_| watch_root.clone())
    };
    // Display label for the report header (mirrors Python's `_report_root_label`
    // at watch.py:125-128 — used as `root` arg to `generate()` at watch.py:516).
    let report_root = report_root_label(watch_path);
    let out = watch_path.join(graphify_out());

    // We currently only use the `code_files` list; the full `DetectResult`
    // will be threaded into the report once analysis carries detection stats.
    let t_detect = std::time::Instant::now();
    let (detected, code_files) = detect_code_files(watch_path, false);
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] detect: {:.2}s ({} files)",
            t_detect.elapsed().as_secs_f64(),
            code_files.len()
        );
    }

    if code_files.is_empty() {
        println!("[graphify watch] No code files found - nothing to rebuild.");
        return Ok(false);
    }

    // Incremental path: only extract changed+still-existing files; track deletions.
    let mut deleted_paths: Vec<String> = Vec::new();
    let extract_targets: Vec<PathBuf>;

    if let Some(changed) = changed_paths {
        let code_set: std::collections::HashSet<PathBuf> = code_files
            .iter()
            .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
            .collect();
        let mut wanted: Vec<PathBuf> = Vec::new();
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
                // Deleted or filtered out — track for node eviction.
                let rel = cand.strip_prefix(&project_root).map_or_else(
                    |_| cand.to_string_lossy().into_owned(),
                    |p| p.to_string_lossy().into_owned(),
                );
                deleted_paths.push(rel);
            }
        }
        if wanted.is_empty() && deleted_paths.is_empty() {
            println!("[graphify watch] No tracked code files in change set - skipping rebuild.");
            return Ok(true);
        }
        extract_targets = wanted;
    } else {
        extract_targets = code_files.clone();
    }

    let commit = git_head(&watch_root);

    // Extract AST nodes.
    let t_extract = std::time::Instant::now();
    let mut result = if extract_targets.is_empty() {
        json!({
            "nodes": [],
            "edges": [],
            "hyperedges": [],
            "input_tokens": 0,
            "output_tokens": 0,
        })
    } else {
        let output = extract(&extract_targets, Some(&watch_root));
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
    let t_post = std::time::Instant::now();

    // Preserve nodes/edges from a prior run, evicting stale entries.
    let existing_graph_path = out.join("graph.json");
    let mut existing_graph_data = Value::Null;

    if existing_graph_path.exists()
        && let Ok(text) = std::fs::read_to_string(&existing_graph_path)
        && let Ok(existing) = serde_json::from_str::<Value>(&text)
    {
        existing_graph_data = existing.clone();

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
        if changed_paths.is_some() {
            for p in &extract_targets {
                let rel = p.strip_prefix(&project_root).map_or_else(
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

        result = json!({
            "nodes": merged_nodes,
            "edges": merged_edges,
            "hyperedges": hyper,
            "input_tokens": 0,
            "output_tokens": 0,
        });
    }

    relativize_source_files(&mut result, &project_root);
    std::fs::create_dir_all(&out).map_err(WatchError::Io)?;
    std::fs::write(
        out.join(".graphify_root"),
        watch_root.to_string_lossy().as_bytes(),
    )
    .map_err(WatchError::Io)?;

    // ── no_cluster path ───────────────────────────────────────────────────────

    if no_cluster {
        // Normalise to "links" key.
        let edges = result
            .get("edges")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![]));
        let mut candidate: serde_json::Map<String, Value> =
            result.as_object().cloned().unwrap_or_default();
        candidate.remove("edges");
        candidate.insert("links".to_string(), edges);
        let candidate_data = Value::Object(candidate);

        let same_graph = if existing_graph_path.exists() {
            if let Ok(text) = std::fs::read_to_string(&existing_graph_path) {
                if let Ok(existing_payload) = serde_json::from_str::<Value>(&text) {
                    let a = serde_json::to_string(&canonical_graph_for_compare(&existing_payload))
                        .unwrap_or_default();
                    let b = serde_json::to_string(&canonical_graph_for_compare(&candidate_data))
                        .unwrap_or_default();
                    a == b
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
            eprintln!(
                "[perf] post-extract canonicalize+compare: {:.2}s",
                t_post.elapsed().as_secs_f64()
            );
        }
        let t_write = std::time::Instant::now();
        if !same_graph {
            check_shrink(force, &existing_graph_data, &candidate_data, None)?;
            std::fs::write(&existing_graph_path, json_text(&candidate_data).as_bytes())
                .map_err(WatchError::Io)?;
        }
        if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
            eprintln!(
                "[perf] write graph.json: {:.2}s",
                t_write.elapsed().as_secs_f64()
            );
        }

        // Clear stale needs_update flag.
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
        return Ok(true);
    }

    // ── full cluster + report path ────────────────────────────────────────────

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

    // Topology comparison: skip full rebuild if structure hasn't changed.
    if !existing_graph_data.is_null() {
        let candidate_topology = graph_to_topology_value(&graph);
        let a = serde_json::to_string(&canonical_topology_for_compare(&existing_graph_data))
            .unwrap_or_default();
        let b = serde_json::to_string(&canonical_topology_for_compare(&candidate_topology))
            .unwrap_or_default();
        if a == b {
            let flag = out.join("needs_update");
            if flag.exists() {
                let _ = std::fs::remove_file(&flag);
            }
            println!(
                "[graphify watch] No code-graph topology changes detected; \
                 outputs left untouched."
            );
            return Ok(true);
        }
    }

    let t_cluster = std::time::Instant::now();
    let previous_community_map = node_community_map(&existing_graph_data);
    let mut communities = cluster(&graph, 1.0, None);
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
    let _cohesion = score_all(&graph, &communities);
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] score_all: {:.2}s",
            t_cohesion.elapsed().as_secs_f64()
        );
    }

    // Labels: start from persistent labels file, fill gaps with defaults.
    let labels_file = out.join(".graphify_labels.json");
    let mut labels: IndexMap<i64, String> = IndexMap::new();
    if labels_file.exists()
        && let Ok(text) = std::fs::read_to_string(&labels_file)
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

    let t_analysis = std::time::Instant::now();
    let mut analysis = build_analysis(&graph, &communities, watch_path);
    // Override the auto-derived "root" with the human-readable label so the
    // report header reads `# Graph Report - {label}` instead of an absolute path.
    if let Some(obj) = analysis.as_object_mut() {
        obj.insert("root".to_string(), Value::String(report_root.clone()));
    }
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] build_analysis: {:.2}s",
            t_analysis.elapsed().as_secs_f64()
        );
    }

    // Write graph to a temp file first, then atomically replace.
    let graph_tmp = out.join(".graph.tmp.json");

    // Attach hyperedges from the extraction result.
    let mut graph_with_hyper = graph.clone();
    if let Some(Value::Array(hyper)) = result.get("hyperedges") {
        attach_hyperedges(&mut graph_with_hyper, hyper);
    }

    let t_to_json = std::time::Instant::now();
    let json_written = to_json(
        &graph_with_hyper,
        &communities,
        &graph_tmp,
        true,
        commit.as_deref(),
    )
    .map_err(|e| WatchError::Pipeline(e.to_string()))?;
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!("[perf] to_json: {:.2}s", t_to_json.elapsed().as_secs_f64());
    }

    if !json_written {
        return Ok(false);
    }

    let candidate_graph_data: Value = if graph_tmp.exists() {
        let text = std::fs::read_to_string(&graph_tmp).map_err(WatchError::Io)?;
        serde_json::from_str(&text).map_err(|e| WatchError::Pipeline(e.to_string()))?
    } else {
        return Ok(false);
    };

    // Compare candidate vs existing to skip unchanged outputs.
    let same_graph = if existing_graph_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&existing_graph_path) {
            if let Ok(existing_payload) = serde_json::from_str::<Value>(&text) {
                let a = serde_json::to_string(&canonical_graph_for_compare(&existing_payload))
                    .unwrap_or_default();
                let b = serde_json::to_string(&canonical_graph_for_compare(&candidate_graph_data))
                    .unwrap_or_default();
                a == b
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    let report_path = out.join("GRAPH_REPORT.md");
    let t_report = std::time::Instant::now();
    let report_content = graphify_report::render_report(&graph_with_hyper, &analysis);
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf] render_report: {:.2}s",
            t_report.elapsed().as_secs_f64()
        );
    }
    let same_report = if report_path.exists() {
        if let Ok(old) = std::fs::read_to_string(&report_path) {
            report_for_compare(&old) == report_for_compare(&report_content)
        } else {
            false
        }
    } else {
        false
    };

    let no_change = same_graph && same_report;

    if no_change {
        let _ = std::fs::remove_file(&graph_tmp);
        println!(
            "[graphify watch] No code-graph changes detected; \
             graph.json/GRAPH_REPORT.md left untouched."
        );
    } else {
        check_shrink(
            force,
            &existing_graph_data,
            &candidate_graph_data,
            Some(&graph_tmp),
        )?;
        // `backup_if_protected` returns the backup path when one was made; we
        // discard it because the rebuild proceeds either way.
        let _ = backup_if_protected(&out);
        std::fs::rename(&graph_tmp, &existing_graph_path).map_err(WatchError::Io)?;
        std::fs::write(&report_path, &report_content).map_err(WatchError::Io)?;

        // Write labels JSON.
        let labels_json_val: serde_json::Map<String, Value> = labels
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.clone())))
            .collect();
        std::fs::write(
            &labels_file,
            json_text(&Value::Object(labels_json_val)).as_bytes(),
        )
        .map_err(WatchError::Io)?;

        // Save the AST-stage manifest so subsequent `extract`/`update` runs
        // can skip files that didn't change.  Mirrors Python's
        // `save_manifest(detected["files"], kind="ast")` at `watch.py:554`.
        let manifest_path = out.join("manifest.json");
        let files_indexed: IndexMap<String, Vec<String>> = detected
            .files
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Err(e) = graphify_detect::save_manifest(&files_indexed, &manifest_path, "ast") {
            println!("[graphify watch] warning: could not write manifest: {e}");
        }
    }

    // Clear stale needs_update flag.
    let flag = out.join("needs_update");
    if flag.exists() {
        let _ = std::fs::remove_file(&flag);
    }

    let mut html_written = false;
    if !no_change {
        let labels_for_html: IndexMap<i64, String> = labels.clone();
        let html_path = out.join("graph.html");
        let t_html = std::time::Instant::now();
        let html_result = to_html(
            &graph_with_hyper,
            &communities,
            &html_path,
            Some(&labels_for_html),
            None,
            None,
        );
        if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
            eprintln!("[perf] to_html: {:.2}s", t_html.elapsed().as_secs_f64());
        }
        match html_result {
            Ok(()) => html_written = true,
            Err(e) => {
                println!("[graphify watch] Skipped graph.html: {e}");
                if html_path.exists() {
                    let _ = std::fs::remove_file(&html_path);
                }
            }
        }
    }

    if !no_change {
        println!(
            "[graphify watch] Rebuilt: {} nodes, {} edges, {} communities",
            graph_with_hyper.node_count(),
            graph_with_hyper.edge_count(),
            communities.len()
        );
        let mut products = String::from("graph.json");
        if html_written {
            products.push_str(", graph.html");
        }
        products.push_str(" and GRAPH_REPORT.md");
        println!("[graphify watch] {products} updated in {}", out.display());
    }

    Ok(true)
}
