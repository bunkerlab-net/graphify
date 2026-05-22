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
#[allow(clippy::fn_params_excessive_bools)]
// reason: mirrors the monolithic extract command from Python's __main__.py:2397;
// each bool maps 1:1 to a distinct CLI flag and collapsing them into an enum
// would diverge from the Python reference.
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
    resolution: f64,
    exclude_hubs: Option<f64>,
    exclude: &[String],
    dedup_llm: bool,
) -> Result<()> {
    if !exclude.is_empty() {
        // Detect crate honours .graphifyignore patterns from disk; programmatic
        // excludes are not yet wired through `detect()`. Surface this so users
        // do not silently rely on the flag.
        eprintln!(
            "warning: --exclude accepted but is currently a no-op \
             (graphify_detect reads patterns from .graphifyignore only)"
        );
    }
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

    // Pre-resolve the output dir + manifest path so we can decide between full
    // detect and incremental detect without rebuilding paths later. Mirrors
    // Python's `incremental_mode = manifest_path.exists() and graph_path.exists()`
    // at `__main__.py:2611`.
    let resolved_out_dir = out.map_or_else(
        || path.join(graphify_out_dir()),
        std::path::Path::to_path_buf,
    );
    let manifest_probe = resolved_out_dir.join("manifest.json");
    let graph_probe = resolved_out_dir.join("graph.json");
    let incremental_mode = manifest_probe.exists() && graph_probe.exists();

    let detect = if incremental_mode {
        eprintln!(
            "[1/6] incremental scan of {} (manifest present) ...",
            path.display()
        );
        // Load the existing manifest, then ask detect_incremental which files
        // changed since the last run. On any I/O error we fall back to a full
        // scan so the user is never blocked.
        let prev = graphify_detect::load_manifest(path).unwrap_or_default();
        match graphify_detect::detect_incremental(path, &prev) {
            Ok(inc) => {
                let new_total: usize = inc.changed_files.values().map(Vec::len).sum();
                let unchanged_total: usize = inc.unchanged_files.values().map(Vec::len).sum();
                eprintln!(
                    "      {new_total} new/changed, {unchanged_total} unchanged, {} deleted",
                    inc.deleted_files.len()
                );
                graphify_detect::detect(path, None, None)
            }
            Err(e) => {
                eprintln!("      incremental scan failed ({e}); falling back to full scan");
                graphify_detect::detect(path, None, None)
            }
        }
    } else {
        eprintln!("[1/6] detecting files in {} ...", path.display());
        graphify_detect::detect(path, None, None)
    };

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
    // Track the LLM output_tokens so we can write `.graphify_semantic_marker`
    // when the run had real LLM content (mirrors Python `__main__.py:2864`).
    let mut sem_output_tokens: u64 = 0;
    let mut sem_input_tokens: u64 = 0;
    let extraction_json = if let Some(ref b) = effective_backend {
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
             (model={}, token-budget={token_budget}, max-concurrency={max_concurrency}) on {} files ...",
            model.unwrap_or("<default>"),
            uncached_files.len()
        );
        let sem_start = std::time::Instant::now();
        let (mut sem_result, failed) =
            graphify_llm::extract_corpus_parallel(&uncached_files, &cfg, None);
        sem_output_tokens = sem_result.output_tokens;
        sem_input_tokens = sem_result.input_tokens;
        eprintln!(
            "      semantic extraction done in {:.1}s \
             ({} nodes, {} edges, {failed} failed chunks)",
            sem_start.elapsed().as_secs_f64(),
            sem_result.nodes.len(),
            sem_result.edges.len(),
        );

        // Save the fresh semantic results into the cache so future runs can
        // skip them. Mirrors Python's `_save_semantic_cache` call.
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

        // Prepend the cached results so they live alongside the fresh ones.
        let mut all_nodes = cache_split.cached_nodes;
        all_nodes.extend(sem_result.nodes);
        sem_result.nodes = all_nodes;
        let mut all_edges = cache_split.cached_edges;
        all_edges.extend(sem_result.edges);
        sem_result.edges = all_edges;
        let mut all_hyper = cache_split.cached_hyperedges;
        all_hyper.extend(sem_result.hyperedges);
        sem_result.hyperedges = all_hyper;
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

    // Drop a `.graphify_root` breadcrumb so `graphify update` invoked from
    // anywhere can recover the original scan root. Mirrors Python's
    // `(out / ".graphify_root").write_text(...)` at `watch.py:424`.
    let scan_root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Err(e) = std::fs::write(
        out_dir.join(".graphify_root"),
        scan_root.to_string_lossy().as_bytes(),
    ) {
        eprintln!("      warning: failed to write .graphify_root: {e}");
    }

    let extraction_path = out_dir.join("stage_02_extract.json");
    std::fs::write(
        &extraction_path,
        serde_json::to_string_pretty(&extraction_json)?,
    )?;
    eprintln!("      wrote {}", extraction_path.display());

    eprintln!("[3/6] building graph ...");
    // Run entity dedup before the graph build so fuzzy duplicates (Jaro-Winkler
    // 92+ on normalised labels, plus MinHash candidate pairs) collapse into one
    // surviving node. Mirrors Python's `build([...], dedup=True, dedup_llm_backend=...)`
    // at `__main__.py:2839` — Python ALWAYS runs dedup on extract.
    let deduped_json = run_entity_dedup(&extraction_json, dedup_llm, effective_backend.as_deref());
    let graph = graphify_build::build_from_json(deduped_json, true, Some(path))?;
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
        let hub_desc = exclude_hubs
            .map(|p| format!(", exclude-hubs={p}"))
            .unwrap_or_default();
        eprintln!(
            "[4/6] clustering (Louvain, resolution={resolution}{hub_desc}) on {} nodes ...",
            graph.node_count()
        );
        let cluster_start = std::time::Instant::now();
        // Python's `--exclude-hubs` takes a 0.0–1.0 fraction; graphify_cluster
        // expects a 0.0–100.0 percentile. Convert here so the CLI surface
        // matches Python byte-for-byte.
        let hubs_pct = exclude_hubs.map(|p| p * 100.0);
        let c = graphify_cluster::cluster(&graph, resolution, hubs_pct);
        eprintln!(
            "      found {} communities in {:.1}s",
            c.len(),
            cluster_start.elapsed().as_secs_f64()
        );
        c
    };
    graphify_export::to_json(&graph, &communities, &graph_path, true, None)?;
    eprintln!("      wrote {}", graph_path.display());

    // Drop a marker so downstream consumers (e.g. wiki export) can tell
    // semantic content was generated. Mirrors `__main__.py:2864`.
    if sem_output_tokens > 0 {
        let marker_path = out_dir.join(".graphify_semantic_marker");
        let marker = serde_json::json!({"output_tokens": sem_output_tokens});
        std::fs::write(&marker_path, serde_json::to_string(&marker)?)?;
    }

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

    // Persist the analysis sidecar so subsequent `graphify export wiki/obsidian/svg/html`
    // invocations can recover communities/cohesion/gods. Mirrors `__main__.py:2889`.
    let analysis_path = out_dir.join(".graphify_analysis.json");
    std::fs::write(&analysis_path, serde_json::to_string_pretty(&analysis)?)?;
    eprintln!("      wrote {}", analysis_path.display());

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

    // Persist a manifest so subsequent `extract`/`update` runs can take the
    // incremental code path (compare file hashes, only re-extract changed files).
    // Mirrors `_save_manifest(... kind="both")` at `__main__.py:2891`. The
    // graphify_detect API expects an IndexMap (deterministic ordering); the
    // detect() result hands us a HashMap so we copy through.
    let manifest_path = out_dir.join("manifest.json");
    let files_indexed: indexmap::IndexMap<String, Vec<String>> = detect
        .files
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Err(e) = graphify_detect::save_manifest(&files_indexed, &manifest_path, "both") {
        eprintln!("      warning: could not write manifest: {e}");
    }

    // Token + cost summary so users can see what the LLM run cost. Python
    // prints this at `__main__.py:2895`. Skip when no LLM was used.
    if (sem_output_tokens > 0 || sem_input_tokens > 0)
        && let Some(b) = effective_backend.as_deref()
    {
        let cost = graphify_llm::estimate_cost(b, sem_input_tokens, sem_output_tokens);
        eprintln!(
            "[graphify extract] tokens: {sem_input_tokens} in / {sem_output_tokens} out (${cost:.4} on {b})"
        );
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
    let ok = graphify_watch::rebuild_code(
        &watch_path,
        None,
        false,
        effective_force,
        no_cluster,
        true, // acquire_lock
        true, // block_on_lock — interactive
    )?;
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
