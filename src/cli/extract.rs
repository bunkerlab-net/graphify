//! `extract` and `update` commands — headless full extraction pipeline
//! (AST + optional LLM semantic enrichment).

use anyhow::Result;

use crate::cli::{build_analysis, graphify_out_dir};

/// Run the headless full extraction pipeline (AST + optional LLM semantic enrichment).
///
/// When a `--backend` is explicitly provided, or when `detect_backend()` finds an
/// LLM API key in the environment, semantic extraction is performed via
/// `graphify_llm::extract_corpus_parallel` and its output is merged on top of the
/// AST nodes/edges via `graphify_llm::merge_into`.  The LLM path is preferred when
/// available because it produces richer relationship types (calls, cites,
/// `conceptually_related_to`, etc.) that the AST extractor cannot infer.
///
/// Ports `__main__.py:2397` (`elif cmd == "extract"`).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
// reason: mirrors the monolithic extract command from Python's __main__.py:2397;
// splitting would fragment the pipeline flow that Python kept as one block.
pub(crate) fn cmd_extract(
    path: &std::path::Path,
    no_cluster: bool,
    out: Option<&std::path::Path>,
    backend: Option<&str>,
    model: Option<&str>,
    max_workers: Option<usize>,
    token_budget: usize,
    max_concurrency: usize,
    api_timeout: u64,
    google_workspace: bool,
    global: bool,
    as_tag: Option<&str>,
) -> Result<()> {
    // `--google-workspace` controls whether Google Drive files are included in
    // detection; the Rust detect crate does not yet expose this flag so we warn.
    if google_workspace {
        eprintln!(
            "warning: --google-workspace accepted but is currently a no-op \
             (graphify_detect does not yet support Google Drive scanning)"
        );
    }
    // `--api-timeout` is accepted for CLI compatibility.  The graphify-llm crate
    // reads `GRAPHIFY_API_TIMEOUT` from the environment; callers who need a custom
    // timeout should set that env var directly.  We do not set it here because
    // `std::env::set_var` is unsafe in multi-threaded contexts (Rust 2024 edition).
    let _ = api_timeout; // suppress unused-variable lint

    // Resolve the effective backend: explicit flag wins; otherwise auto-detect from
    // environment (mirrors Python's `_detect_backend()` at llm.py).
    let effective_backend: Option<String> = backend
        .map(str::to_string)
        .or_else(graphify_llm::detect_backend);

    let start = std::time::Instant::now();

    eprintln!("[1/6] detecting files in {} ...", path.display());
    let detect = graphify_detect::detect(path, None, None);
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut by_kind: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (kind, paths) in &detect.files {
        by_kind.insert(kind.as_str(), paths.len());
        if kind == "code" || kind == "document" {
            for p in paths {
                files.push(path.join(p));
            }
        }
    }
    let kinds_summary = by_kind
        .iter()
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "      detected {} files ({kinds_summary}); will extract from {}",
        detect.total_files,
        files.len()
    );

    eprintln!("[2/6] extracting AST from {} files ...", files.len());
    let extract_start = std::time::Instant::now();
    let extraction = graphify_extract::extract(&files, Some(path));
    eprintln!(
        "      extracted {} nodes, {} edges in {:.1}s",
        extraction.nodes.len(),
        extraction.edges.len(),
        extract_start.elapsed().as_secs_f64()
    );

    // When an LLM backend is available, run semantic extraction and merge the
    // results on top of the AST output.  Semantic nodes/edges take priority
    // because they carry richer relation types (calls, cites, etc.) that the
    // AST extractor cannot infer.  This matches Python's two-pass strategy at
    // __main__.py:2500–2560.
    //
    // `extraction.nodes`/`edges` are `Vec<IndexMap<String, Value>>` (graphify-extract
    // types).  `sem_result.nodes`/`edges` are `Vec<serde_json::Value>` (graphify-llm
    // types).  We convert by serializing via serde_json so the final JSON is uniform.
    let extraction_json = if let Some(ref b) = effective_backend {
        let chunk_size = max_workers.unwrap_or(8);
        let cfg = graphify_llm::CorpusConfig {
            backend: b.as_str(),
            api_key: None,
            model: model.filter(|s| !s.is_empty()),
            root: path,
            chunk_size,
            token_budget: Some(token_budget),
            max_concurrency,
            max_retry_depth: 3,
        };
        eprintln!(
            "      running LLM semantic extraction via backend={b} \
             (model={}, token-budget={token_budget}, max-concurrency={max_concurrency}) ...",
            model.unwrap_or("<default>")
        );
        let sem_start = std::time::Instant::now();
        let (mut sem_result, failed) = graphify_llm::extract_corpus_parallel(&files, &cfg, None);
        eprintln!(
            "      semantic extraction done in {:.1}s \
             ({} nodes, {} edges, {failed} failed chunks)",
            sem_start.elapsed().as_secs_f64(),
            sem_result.nodes.len(),
            sem_result.edges.len(),
        );
        // Append AST nodes/edges after the semantic ones; semantic entries win
        // any deduplication that happens inside build_from_json.
        // Convert IndexMap→Value via JSON round-trip (both serialize identically).
        if let serde_json::Value::Array(ast_nodes_v) =
            serde_json::to_value(&extraction.nodes).unwrap_or(serde_json::Value::Array(vec![]))
        {
            sem_result.nodes.extend(ast_nodes_v);
        }
        if let serde_json::Value::Array(ast_edges_v) =
            serde_json::to_value(&extraction.edges).unwrap_or(serde_json::Value::Array(vec![]))
        {
            sem_result.edges.extend(ast_edges_v);
        }
        serde_json::json!({
            "nodes": sem_result.nodes,
            "edges": sem_result.edges,
            "hyperedges": sem_result.hyperedges,
        })
    } else {
        serde_json::json!({
            "nodes": extraction.nodes,
            "edges": extraction.edges,
            "hyperedges": [],
        })
    };
    // Resolve output dir. When `--out` is not set, use GRAPHIFY_OUT (defaulting
    // to "graphify-out") relative to the source path, matching Python's contract.
    let out_dir = out.map_or_else(
        || path.join(graphify_out_dir()),
        std::path::Path::to_path_buf,
    );
    std::fs::create_dir_all(&out_dir)?;
    let extraction_path = out_dir.join("stage_02_extract.json");
    std::fs::write(
        &extraction_path,
        serde_json::to_string_pretty(&extraction_json)?,
    )?;
    eprintln!("      wrote {}", extraction_path.display());

    eprintln!("[3/6] building graph ...");
    let graph = graphify_build::build_from_json(extraction_json, true, Some(path))?;
    eprintln!(
        "      built graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    let graph_path = out_dir.join("graph.json");

    let communities = if no_cluster {
        eprintln!("[4/6] clustering: skipped (--no-cluster)");
        indexmap::IndexMap::new()
    } else {
        eprintln!(
            "[4/6] clustering (Louvain, resolution=1.0) on {} nodes ...",
            graph.node_count()
        );
        let cluster_start = std::time::Instant::now();
        let c = graphify_cluster::cluster(&graph, 1.0, None);
        eprintln!(
            "      found {} communities in {:.1}s",
            c.len(),
            cluster_start.elapsed().as_secs_f64()
        );
        c
    };
    graphify_export::to_json(&graph, &communities, &graph_path, true, None)?;
    eprintln!("      wrote {}", graph_path.display());

    if no_cluster {
        // Wire --global/--as even when clustering is skipped (mirrors Python).
        if global {
            cmd_extract_global_add(&graph_path, as_tag, path);
        }
        eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
        return Ok(());
    }

    eprintln!("[5/6] analyzing (god nodes, surprising connections, suggested questions) ...");
    let analyze_start = std::time::Instant::now();
    let analysis = build_analysis(&graph, &communities, path);
    eprintln!(
        "      analysis done in {:.1}s",
        analyze_start.elapsed().as_secs_f64()
    );
    let report_path = out_dir.join("GRAPH_REPORT.md");
    graphify_report::write_report(&graph, &analysis, &report_path)?;
    eprintln!("      wrote {}", report_path.display());

    eprintln!("[6/6] rendering HTML viz ...");
    let html_path = out_dir.join("graph.html");
    match graphify_export::to_html(&graph, &communities, &html_path, None, None, None) {
        Ok(()) => eprintln!("      wrote {}", html_path.display()),
        Err(e) => eprintln!("      skipped ({e})"),
    }

    // Wire --global/--as after extract+cluster+report, mirroring Python's
    // `global_add` call at __main__.py:2867.
    if global {
        cmd_extract_global_add(&graph_path, as_tag, path);
    }

    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    Ok(())
}

/// Re-extract code files and update the graph (no LLM re-run by default).
///
/// Thin wrapper over `cmd_extract` with all LLM flags disabled.  The `force`
/// flag is accepted for CLI compatibility but is currently a no-op in the Rust
/// port; future versions may use it to skip the incremental-cache check.
pub(crate) fn cmd_update(path: &std::path::Path, force: bool, no_cluster: bool) -> Result<()> {
    // `update` is a thin wrapper over `extract` with defaults for all LLM flags.
    cmd_extract(
        path, no_cluster, None, None, None, None, 60_000, 4, 600, false, false, None,
    )?;
    let _ = force;
    Ok(())
}

/// Merge the just-written graph into the global graph.
///
/// Mirrors `_global_add(graphify_out / "graph.json", _tag)` from Python's
/// extract path at `__main__.py:2867`. Errors are non-fatal: a global graph
/// failure should never abort the local extraction.
pub(crate) fn cmd_extract_global_add(
    graph_path: &std::path::Path,
    as_tag: Option<&str>,
    project_root: &std::path::Path,
) {
    let tag = as_tag.map_or_else(
        || {
            project_root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        },
        str::to_string,
    );
    let manifest_path = graphify_global::global_manifest_path();
    let global_path = graphify_global::global_graph_path();
    match graphify_global::global_add(graph_path, &tag, &global_path, &manifest_path) {
        Ok(summary) => {
            if summary.nodes_added == 0 && summary.nodes_removed == 0 {
                eprintln!("[graphify global] '{tag}' unchanged since last add — skipped.");
            } else {
                eprintln!(
                    "[graphify global] '{tag}' merged into global graph \
                     (+{} nodes, -{} pruned).",
                    summary.nodes_added, summary.nodes_removed
                );
            }
        }
        Err(e) => {
            eprintln!("[graphify global] warning: failed to merge into global graph: {e}");
        }
    }
}
