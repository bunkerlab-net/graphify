//! `extract` and `update` commands — headless full extraction pipeline
//! (AST + optional LLM semantic enrichment).

use anyhow::Result;

use crate::cli::{build_analysis, graphify_out_dir};

/// Knobs for the LLM semantic-extraction phase.
pub(crate) struct LlmOptions<'a> {
    pub backend: Option<&'a str>,
    pub model: Option<&'a str>,
    pub max_workers: Option<usize>,
    pub token_budget: usize,
    pub max_concurrency: usize,
    pub api_timeout: u64,
    pub dedup_llm: bool,
}

/// Clustering / hub-exclusion knobs.
pub(crate) struct ClusterOptions {
    pub no_cluster: bool,
    pub resolution: f64,
    pub exclude_hubs: Option<f64>,
}

/// Knobs for "promote into the global graph" behaviour.
pub(crate) struct GlobalOptions<'a> {
    pub global: bool,
    pub as_tag: Option<&'a str>,
}

/// Aggregated arguments for [`cmd_extract`].
pub(crate) struct ExtractOptions<'a> {
    pub path: &'a std::path::Path,
    pub out: Option<&'a std::path::Path>,
    pub exclude: &'a [String],
    pub google_workspace: bool,
    pub llm: LlmOptions<'a>,
    pub cluster: ClusterOptions,
    pub global: GlobalOptions<'a>,
}

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
pub(crate) fn cmd_extract(opts: ExtractOptions<'_>) -> Result<()> {
    let ExtractOptions {
        path,
        out,
        exclude,
        google_workspace,
        llm,
        cluster,
        global,
    } = opts;
    let LlmOptions {
        backend,
        model,
        max_workers,
        token_budget,
        max_concurrency,
        api_timeout,
        dedup_llm,
    } = llm;
    let ClusterOptions {
        no_cluster,
        resolution,
        exclude_hubs,
    } = cluster;
    let GlobalOptions { global, as_tag } = global;

    let extra_excludes: Option<&[String]> = if exclude.is_empty() {
        None
    } else {
        Some(exclude)
    };
    apply_env_overrides(api_timeout, google_workspace);

    // Resolve the effective backend: explicit flag wins; otherwise auto-detect from
    // environment (mirrors Python's `_detect_backend()` at llm.py).
    let effective_backend: Option<String> = backend
        .map(str::to_string)
        .or_else(graphify_llm::detect_backend);

    let start = std::time::Instant::now();
    let out_dir = out.map_or_else(
        || path.join(graphify_out_dir()),
        std::path::Path::to_path_buf,
    );

    let detect = run_detect_phase(path, &out_dir, extra_excludes);
    let files = collect_extract_files(path, &detect);
    let extraction = run_ast_extract_phase(&files, path);
    let cfg = SemanticConfig {
        backend: effective_backend.as_deref(),
        model,
        max_workers,
        token_budget,
        max_concurrency,
    };
    let SemanticOutcome {
        extraction_json,
        sem_input_tokens,
        sem_output_tokens,
    } = run_semantic_phase(path, &files, &extraction, &cfg);

    std::fs::create_dir_all(&out_dir)?;
    write_scan_breadcrumb(path, &out_dir);
    persist_raw_extraction(&out_dir, &extraction_json)?;

    let graph = build_graph_phase(
        &extraction_json,
        dedup_llm,
        effective_backend.as_deref(),
        path,
    )?;
    let graph_path = out_dir.join("graph.json");
    let communities = run_cluster_phase(&graph, no_cluster, resolution, exclude_hubs);
    graphify_export::to_json(&graph, &communities, &graph_path, true, None)?;
    eprintln!("      wrote {}", graph_path.display());
    persist_semantic_marker(&out_dir, sem_output_tokens)?;

    if no_cluster {
        if global {
            cmd_extract_global_add(&graph_path, as_tag, path);
        }
        eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
        return Ok(());
    }

    run_analysis_phase(&graph, &communities, path, &out_dir)?;
    let labels = sync_labels_file(&out_dir, &communities)?;
    render_html_viz(&graph, &communities, &out_dir, &labels);

    if global {
        cmd_extract_global_add(&graph_path, as_tag, path);
    }
    persist_manifest(&detect.files, &out_dir);
    print_token_summary(
        effective_backend.as_deref(),
        sem_input_tokens,
        sem_output_tokens,
    );

    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    Ok(())
}

/// Detect phase: incremental scan when a manifest is present, otherwise a full scan.
fn run_detect_phase(
    path: &std::path::Path,
    resolved_out_dir: &std::path::Path,
    extra_excludes: Option<&[String]>,
) -> graphify_detect::DetectResult {
    // Mirrors Python's `incremental_mode = manifest_path.exists() and graph_path.exists()`
    // at `__main__.py:2611`.
    let incremental_mode = resolved_out_dir.join("manifest.json").exists()
        && resolved_out_dir.join("graph.json").exists();
    if incremental_mode {
        eprintln!(
            "[1/6] incremental scan of {} (manifest present) ...",
            path.display()
        );
        let prev = graphify_detect::load_manifest(path).unwrap_or_default();
        match graphify_detect::detect_incremental(path, &prev) {
            Ok(inc) => {
                let new_total: usize = inc.changed_files.values().map(Vec::len).sum();
                let unchanged_total: usize = inc.unchanged_files.values().map(Vec::len).sum();
                eprintln!(
                    "      {new_total} new/changed, {unchanged_total} unchanged, {} deleted",
                    inc.deleted_files.len()
                );
                return detect_result_from_incremental(path, &inc);
            }
            Err(e) => eprintln!("      incremental scan failed ({e}); falling back to full scan"),
        }
        graphify_detect::detect(path, None, extra_excludes)
    } else {
        eprintln!("[1/6] detecting files in {} ...", path.display());
        graphify_detect::detect(path, None, extra_excludes)
    }
}

/// Convert an [`IncrementalDetectResult`] back into a [`DetectResult`] using
/// the union of changed + unchanged files. Saves the redundant `detect`
/// walk that used to follow every successful incremental scan.
fn detect_result_from_incremental(
    path: &std::path::Path,
    inc: &graphify_detect::IncrementalDetectResult,
) -> graphify_detect::DetectResult {
    let mut files: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (kind, paths) in &inc.changed_files {
        files.entry(kind.clone()).or_default().extend(paths.clone());
    }
    for (kind, paths) in &inc.unchanged_files {
        files.entry(kind.clone()).or_default().extend(paths.clone());
    }
    let total_files = files.values().map(Vec::len).sum();
    graphify_detect::DetectResult {
        files,
        total_files,
        total_words: 0,
        needs_graph: true,
        warning: None,
        skipped_sensitive: Vec::new(),
        graphifyignore_patterns: 0,
        scan_root: path.to_string_lossy().into_owned(),
    }
}

/// Flatten the "code" + "document" buckets into absolute paths and print a summary.
fn collect_extract_files(
    path: &std::path::Path,
    detect: &graphify_detect::DetectResult,
) -> Vec<std::path::PathBuf> {
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
    files
}

/// AST extraction stage (step [2/6]).
fn run_ast_extract_phase(
    files: &[std::path::PathBuf],
    path: &std::path::Path,
) -> graphify_extract::ExtractOutput {
    eprintln!("[2/6] extracting AST from {} files ...", files.len());
    let extract_start = std::time::Instant::now();
    let extraction = graphify_extract::extract(files, Some(path));
    eprintln!(
        "      extracted {} nodes, {} edges in {:.1}s",
        extraction.nodes.len(),
        extraction.edges.len(),
        extract_start.elapsed().as_secs_f64()
    );
    extraction
}

/// Sub-options needed to drive [`run_semantic_phase`].
struct SemanticConfig<'a> {
    backend: Option<&'a str>,
    model: Option<&'a str>,
    max_workers: Option<usize>,
    token_budget: usize,
    max_concurrency: usize,
}

/// Output of [`run_semantic_phase`]: merged JSON plus token totals.
struct SemanticOutcome {
    extraction_json: serde_json::Value,
    sem_input_tokens: u64,
    sem_output_tokens: u64,
}

/// Optional LLM semantic-extraction stage, merging onto the AST result.
///
/// When no backend is configured, returns the AST extraction as-is.
fn run_semantic_phase(
    path: &std::path::Path,
    files: &[std::path::PathBuf],
    extraction: &graphify_extract::ExtractOutput,
    cfg: &SemanticConfig<'_>,
) -> SemanticOutcome {
    let Some(b) = cfg.backend else {
        let extraction_json = serde_json::json!({
            "nodes": extraction.nodes,
            "edges": extraction.edges,
            "hyperedges": [],
        });
        return SemanticOutcome {
            extraction_json,
            sem_input_tokens: 0,
            sem_output_tokens: 0,
        };
    };

    // Semantic cache check — skip files already extracted to avoid re-spending
    // LLM tokens on the same content. Mirrors Python `__main__.py:2682`.
    let sem_paths: Vec<String> = files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let cache_split = graphify_cache::check_semantic_cache(&sem_paths, path);
    let cache_hits = sem_paths
        .len()
        .saturating_sub(cache_split.uncached_files.len());
    if cache_hits > 0 {
        eprintln!(
            "      semantic cache: {cache_hits} hit / {} miss",
            cache_split.uncached_files.len()
        );
    }
    let uncached_files: Vec<std::path::PathBuf> = cache_split
        .uncached_files
        .iter()
        .map(std::path::PathBuf::from)
        .collect();

    let chunk_size = cfg.max_workers.unwrap_or(8);
    let llm_cfg = graphify_llm::CorpusConfig {
        backend: b,
        api_key: None,
        model: cfg.model.filter(|s| !s.is_empty()),
        root: path,
        chunk_size,
        token_budget: Some(cfg.token_budget),
        max_concurrency: cfg.max_concurrency,
        max_retry_depth: 3,
    };
    eprintln!(
        "      running LLM semantic extraction via backend={b} \
         (model={}, token-budget={}, max-concurrency={}) on {} files ...",
        cfg.model.unwrap_or("<default>"),
        cfg.token_budget,
        cfg.max_concurrency,
        uncached_files.len()
    );
    let sem_start = std::time::Instant::now();
    let (mut sem_result, failed, total_chunks) =
        graphify_llm::extract_corpus_parallel_with_total(&uncached_files, &llm_cfg, None);
    let sem_output_tokens = sem_result.output_tokens;
    let sem_input_tokens = sem_result.input_tokens;
    eprintln!(
        "      semantic extraction done in {:.1}s \
         ({} nodes, {} edges, {failed} failed chunks)",
        sem_start.elapsed().as_secs_f64(),
        sem_result.nodes.len(),
        sem_result.edges.len(),
    );

    // When every chunk failed, exit non-zero rather than silently writing
    // an AST-only graph. Mirrors graphify-py `__main__.py:_chunk_stats`
    // ("all semantic chunks failed ... claude" exit path).
    if !uncached_files.is_empty() && total_chunks > 0 && failed >= total_chunks {
        eprintln!(
            "error: all semantic chunks failed (backend={b}). \
             Check your API key and that `pip install \"graphifyy[mcp]\"` (or the equivalent \
             Cargo install) is up to date."
        );
        std::process::exit(1);
    }

    save_semantic_cache_safe(&sem_result, path);
    merge_semantic_with_cache_and_ast(&mut sem_result, cache_split, extraction);
    let extraction_json = serde_json::json!({
        "nodes": sem_result.nodes,
        "edges": sem_result.edges,
        "hyperedges": sem_result.hyperedges,
    });
    SemanticOutcome {
        extraction_json,
        sem_input_tokens,
        sem_output_tokens,
    }
}

/// Best-effort persistence of fresh semantic results into the cache. Warns on
/// I/O failure instead of bubbling the error up.
fn save_semantic_cache_safe(sem_result: &graphify_llm::LlmResponse, path: &std::path::Path) {
    if (!sem_result.nodes.is_empty() || !sem_result.edges.is_empty())
        && let Err(e) = graphify_cache::save_semantic_cache(
            &sem_result.nodes,
            &sem_result.edges,
            &sem_result.hyperedges,
            path,
        )
    {
        eprintln!("      warning: failed to save semantic cache: {e}");
    }
}

/// Prepend cached semantic entries to `sem_result`, then append AST nodes/edges.
///
/// Semantic entries win any deduplication that happens inside `build_from_json`.
fn merge_semantic_with_cache_and_ast(
    sem_result: &mut graphify_llm::LlmResponse,
    cache_split: graphify_cache::SemanticCacheSplit,
    extraction: &graphify_extract::ExtractOutput,
) {
    let mut all_nodes = cache_split.cached_nodes;
    all_nodes.extend(std::mem::take(&mut sem_result.nodes));
    sem_result.nodes = all_nodes;
    let mut all_edges = cache_split.cached_edges;
    all_edges.extend(std::mem::take(&mut sem_result.edges));
    sem_result.edges = all_edges;
    let mut all_hyper = cache_split.cached_hyperedges;
    all_hyper.extend(std::mem::take(&mut sem_result.hyperedges));
    sem_result.hyperedges = all_hyper;
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
}

/// Drop a `.graphify_root` breadcrumb so `graphify update` invoked from any
/// directory can recover the original scan root.
fn write_scan_breadcrumb(path: &std::path::Path, out_dir: &std::path::Path) {
    let scan_root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Err(e) = std::fs::write(
        out_dir.join(".graphify_root"),
        scan_root.to_string_lossy().as_bytes(),
    ) {
        eprintln!("      warning: failed to write .graphify_root: {e}");
    }
}

/// Persist the raw extraction (AST + semantic) JSON sidecar.
fn persist_raw_extraction(
    out_dir: &std::path::Path,
    extraction_json: &serde_json::Value,
) -> Result<()> {
    let extraction_path = out_dir.join("stage_02_extract.json");
    std::fs::write(
        &extraction_path,
        serde_json::to_string_pretty(extraction_json)?,
    )?;
    eprintln!("      wrote {}", extraction_path.display());
    Ok(())
}

/// Build stage (step [3/6]): entity dedup + `build_from_json`.
fn build_graph_phase(
    extraction_json: &serde_json::Value,
    dedup_llm: bool,
    backend: Option<&str>,
    path: &std::path::Path,
) -> Result<graphify_build::Graph> {
    eprintln!("[3/6] building graph ...");
    let deduped_json = run_entity_dedup(extraction_json, dedup_llm, backend);
    let graph = graphify_build::build_from_json(deduped_json, true, Some(path))?;
    eprintln!(
        "      built graph: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );
    Ok(graph)
}

/// Cluster stage (step [4/6]) — returns an empty map when clustering is skipped.
fn run_cluster_phase(
    graph: &graphify_build::Graph,
    no_cluster: bool,
    resolution: f64,
    exclude_hubs: Option<f64>,
) -> indexmap::IndexMap<i64, Vec<String>> {
    if no_cluster {
        eprintln!("[4/6] clustering: skipped (--no-cluster)");
        return indexmap::IndexMap::new();
    }
    let hub_desc = exclude_hubs
        .map(|p| format!(", exclude-hubs={p}"))
        .unwrap_or_default();
    eprintln!(
        "[4/6] clustering (Louvain, resolution={resolution}{hub_desc}) on {} nodes ...",
        graph.node_count()
    );
    let cluster_start = std::time::Instant::now();
    // Python's `--exclude-hubs` takes a 0.0–1.0 fraction; graphify_cluster expects
    // a 0.0–100.0 percentile. Convert here so the CLI surface matches Python.
    let hubs_pct = exclude_hubs.map(|p| p * 100.0);
    let c = graphify_cluster::cluster(graph, resolution, hubs_pct);
    eprintln!(
        "      found {} communities in {:.1}s",
        c.len(),
        cluster_start.elapsed().as_secs_f64()
    );
    c
}

/// Drop a `.graphify_semantic_marker` so downstream consumers (e.g. wiki export)
/// can tell semantic content was generated. Mirrors `__main__.py:2864`.
fn persist_semantic_marker(out_dir: &std::path::Path, sem_output_tokens: u64) -> Result<()> {
    if sem_output_tokens > 0 {
        let marker_path = out_dir.join(".graphify_semantic_marker");
        let marker = serde_json::json!({"output_tokens": sem_output_tokens});
        std::fs::write(&marker_path, serde_json::to_string(&marker)?)?;
    }
    Ok(())
}

/// Analyze stage (step [5/6]): god nodes + surprises + suggested questions + sidecar.
fn run_analysis_phase(
    graph: &graphify_build::Graph,
    communities: &indexmap::IndexMap<i64, Vec<String>>,
    path: &std::path::Path,
    out_dir: &std::path::Path,
) -> Result<()> {
    eprintln!("[5/6] analyzing (god nodes, surprising connections, suggested questions) ...");
    let analyze_start = std::time::Instant::now();
    let analysis = build_analysis(graph, communities, path);
    eprintln!(
        "      analysis done in {:.1}s",
        analyze_start.elapsed().as_secs_f64()
    );
    let report_path = out_dir.join("GRAPH_REPORT.md");
    graphify_report::write_report(graph, &analysis, &report_path)?;
    eprintln!("      wrote {}", report_path.display());

    let analysis_path = out_dir.join(".graphify_analysis.json");
    std::fs::write(&analysis_path, serde_json::to_string_pretty(&analysis)?)?;
    eprintln!("      wrote {}", analysis_path.display());
    Ok(())
}

/// Load `.graphify_labels.json`, top up missing entries with `"Community <cid>"`,
/// and write the merged map back. Preserves user-edited names.
fn sync_labels_file(
    out_dir: &std::path::Path,
    communities: &indexmap::IndexMap<i64, Vec<String>>,
) -> Result<indexmap::IndexMap<i64, String>> {
    let labels_path = out_dir.join(".graphify_labels.json");
    let mut labels: indexmap::IndexMap<i64, String> = indexmap::IndexMap::new();
    if let Ok(text) = std::fs::read_to_string(&labels_path)
        && let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text)
    {
        for (k, v) in &map {
            if let (Ok(cid), Some(s)) = (k.parse::<i64>(), v.as_str())
                && communities.contains_key(&cid)
            {
                labels.insert(cid, s.to_string());
            }
        }
    }
    for cid in communities.keys() {
        labels
            .entry(*cid)
            .or_insert_with(|| format!("Community {cid}"));
    }
    let labels_json: serde_json::Map<String, serde_json::Value> = labels
        .iter()
        .map(|(cid, name)| (cid.to_string(), serde_json::Value::String(name.clone())))
        .collect();
    std::fs::write(
        &labels_path,
        serde_json::to_string(&serde_json::Value::Object(labels_json))?,
    )?;
    eprintln!("      wrote {}", labels_path.display());
    Ok(labels)
}

/// HTML viz stage (step [6/6]).
fn render_html_viz(
    graph: &graphify_build::Graph,
    communities: &indexmap::IndexMap<i64, Vec<String>>,
    out_dir: &std::path::Path,
    labels: &indexmap::IndexMap<i64, String>,
) {
    eprintln!("[6/6] rendering HTML viz ...");
    let html_path = out_dir.join("graph.html");
    let labels_opt = if labels.is_empty() {
        None
    } else {
        Some(labels)
    };
    match graphify_export::to_html(graph, communities, &html_path, labels_opt, None, None) {
        Ok(()) => eprintln!("      wrote {}", html_path.display()),
        Err(e) => eprintln!("      skipped ({e})"),
    }
}

/// Persist a manifest so subsequent `extract`/`update` runs can take the
/// incremental code path. Mirrors `_save_manifest(... kind="both")` at
/// `__main__.py:2891`.
fn persist_manifest(
    detect_files: &std::collections::HashMap<String, Vec<String>>,
    out_dir: &std::path::Path,
) {
    let manifest_path = out_dir.join("manifest.json");
    let files_indexed: indexmap::IndexMap<String, Vec<String>> = detect_files
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Err(e) = graphify_detect::save_manifest(&files_indexed, &manifest_path, "both") {
        eprintln!("      warning: could not write manifest: {e}");
    }
}

/// Print the token + cost summary so users can see what the LLM run cost.
/// Mirrors Python's `__main__.py:2895`. No-op when no LLM was used.
fn print_token_summary(backend: Option<&str>, sem_input_tokens: u64, sem_output_tokens: u64) {
    if (sem_output_tokens > 0 || sem_input_tokens > 0)
        && let Some(b) = backend
    {
        let cost = graphify_llm::estimate_cost(b, sem_input_tokens, sem_output_tokens);
        eprintln!(
            "[graphify extract] tokens: {sem_input_tokens} in / {sem_output_tokens} out (${cost:.4} on {b})"
        );
    }
}

/// Apply per-run environment overrides for the LLM client and Google Workspace.
///
/// Mirrors Python `__main__.py:2518-2519` (API timeout) and `__main__.py:2479-2480`
/// (Google Workspace).  Called at the top of `cmd_extract`, before any LLM worker
/// threads spawn, so the SAFETY contract on `std::env::set_var` (no concurrent env
/// reads/writes) holds.
fn apply_env_overrides(api_timeout: u64, google_workspace: bool) {
    if api_timeout > 0 {
        // SAFETY: cmd_extract runs on the single-threaded main runtime; no other
        // thread reads or writes the environment at this point.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("GRAPHIFY_API_TIMEOUT", api_timeout.to_string());
        }
    }
    if google_workspace {
        // SAFETY: see above — set before any worker thread is spawned.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("GRAPHIFY_GOOGLE_WORKSPACE", "1");
        }
    }
}

/// Re-extract code files and update the graph (no LLM re-run by default).
///
/// Thin wrapper over `cmd_extract` with all LLM flags disabled.  `force` is
/// forwarded to `graphify_watch::rebuild_code` where it overrides the
/// shrink-guard so a rebuild with fewer nodes is allowed (mirrors Python's
/// `--force` and `GRAPHIFY_FORCE=1` at `__main__.py:1854,1860`).
pub(crate) fn cmd_update(path: &std::path::Path, force: bool, no_cluster: bool) -> Result<()> {
    // Mirror Python's `update` at `__main__.py:1853`: AST-only rebuild via the
    // watch crate's `rebuild_code`, blocking on the lock so an interactive
    // `graphify update` always completes (instead of skipping when a hook is
    // already rebuilding). Recovers the scan root from `.graphify_root` when
    // the user invoked update from inside graphify-out instead of the project.
    let env_force = std::env::var("GRAPHIFY_FORCE")
        .ok()
        .is_some_and(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"));
    let effective_force = force || env_force;

    let watch_path: std::path::PathBuf = if path.exists() {
        path.to_path_buf()
    } else {
        // Try to recover from a previously written `.graphify_root` file.
        let saved = crate::cli::graphify_out_dir().join(".graphify_root");
        if let Ok(text) = std::fs::read_to_string(&saved) {
            std::path::PathBuf::from(text.trim())
        } else {
            anyhow::bail!("path not found: {}", path.display());
        }
    };

    eprintln!(
        "Re-extracting code files in {} (no LLM needed)...",
        watch_path.display()
    );
    let opts = graphify_watch::RebuildOptions {
        force: effective_force,
        no_cluster,
        lock: graphify_watch::LockPolicy::BlockOn, // interactive
    };
    let ok = graphify_watch::rebuild_code(&watch_path, None, opts)?;
    if ok {
        println!(
            "Code graph updated. For doc/paper/image changes run /graphify --update in your AI assistant."
        );
        if std::env::var("GEMINI_API_KEY").is_err()
            && std::env::var("GOOGLE_API_KEY").is_err()
            && std::env::var("MOONSHOT_API_KEY").is_err()
            && std::env::var("DEEPSEEK_API_KEY").is_err()
            && std::env::var("GRAPHIFY_NO_TIPS").is_err()
        {
            println!(
                "Tip: set GEMINI_API_KEY or GOOGLE_API_KEY to use Gemini for semantic extraction."
            );
        }
        Ok(())
    } else {
        anyhow::bail!("Nothing to update or rebuild failed — check output above.")
    }
}

/// `DedupLlmBackend` that calls the configured graphify-llm backend on each
/// ambiguous Jaro-Winkler-tied pair.
///
/// Mirrors Python's `_llm_tiebreak` at `graphify-py/graphify/dedup.py:322`
/// (single-pair flavour). Returns `Merge` only for an explicit positive YES
/// from the model so dedup stays conservative; anything else becomes
/// `Distinct` (no merge).
struct LlmDedupBackend {
    backend: String,
}

impl graphify_dedup::DedupLlmBackend for LlmDedupBackend {
    fn judge(&self, a: &str, b: &str) -> graphify_dedup::JudgeResult {
        let prompt = format!(
            "You are deciding whether two entity labels refer to the same real-world \
             concept in a knowledge graph. Respond with exactly one word: YES if they \
             are the same concept, NO if distinct, UNSURE if you cannot tell.\n\n\
             Label A: {a}\nLabel B: {b}\n\nAnswer:"
        );
        match graphify_llm::call_llm(&prompt, &self.backend, 8) {
            Ok(raw) => {
                let answer = raw.trim().to_uppercase();
                if answer.starts_with("YES") {
                    graphify_dedup::JudgeResult::Merge
                } else if answer.starts_with("NO") {
                    graphify_dedup::JudgeResult::Distinct
                } else {
                    graphify_dedup::JudgeResult::Uncertain
                }
            }
            Err(_) => graphify_dedup::JudgeResult::Uncertain,
        }
    }
}

/// Run entity deduplication on an extraction JSON value.
///
/// Always runs the label-canonical pass (cheap). When `dedup_llm` is true
/// and a backend is detected, also runs the Jaro-Winkler tiebreaker with
/// `LlmDedupBackend`. Mirrors Python's `build(..., dedup=True, dedup_llm_backend=...)`.
fn run_entity_dedup(
    extraction: &serde_json::Value,
    dedup_llm: bool,
    backend: Option<&str>,
) -> serde_json::Value {
    use serde_json::Value;
    let empty: Vec<Value> = Vec::new();
    let nodes: &[Value] = extraction
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(&empty[..], Vec::as_slice);
    let edges: &[Value] = extraction
        .get("edges")
        .or_else(|| extraction.get("links"))
        .and_then(Value::as_array)
        .map_or(&empty[..], Vec::as_slice);
    let hyperedges = extraction
        .get("hyperedges")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));

    let communities: indexmap::IndexMap<String, i64> = indexmap::IndexMap::new();
    let result = if dedup_llm && let Some(b) = backend {
        let backend_obj = LlmDedupBackend {
            backend: b.to_string(),
        };
        graphify_dedup::deduplicate_entities(nodes, edges, &communities, Some(&backend_obj))
    } else {
        graphify_dedup::deduplicate_entities(nodes, edges, &communities, None)
    };

    let (deduped_nodes, deduped_edges) = match result {
        Ok(pair) => pair,
        Err(e) => {
            // Dedup is best-effort: a multi-repo guard or empty-group surprise
            // should never abort extraction. Fall back to the raw extraction.
            eprintln!("      warning: dedup skipped ({e})");
            (nodes.to_vec(), edges.to_vec())
        }
    };

    serde_json::json!({
        "nodes": deduped_nodes,
        "edges": deduped_edges,
        "hyperedges": hyperedges,
    })
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
