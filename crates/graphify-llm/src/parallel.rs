//! Corpus-parallel extraction with chunk merging.
//!
//! Extracted from `lib.rs` to isolate `extract_corpus_parallel` and
//! `merge_into` — the Rayon-backed fan-out that processes multiple file
//! chunks concurrently and assembles a single merged `LlmResponse`.

use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use crate::file_slice::{Unit, expand_oversized_files, unit_path};
use crate::retry::{AdaptiveRetryCtx, extract_with_adaptive_retry_units};
use crate::tokens::pack_chunks_by_tokens_units;
use crate::{FILE_CHAR_CAP, LlmResponse};

/// Merge a chunk result into the running accumulator in-place.
///
/// Appends `result.nodes`, `edges`, and `hyperedges` to `merged` and
/// accumulates token counts and elapsed time. Called after every successful
/// chunk in `extract_corpus_parallel` to build the final merged response.
pub fn merge_into(merged: &mut LlmResponse, result: &LlmResponse) {
    merged.nodes.extend_from_slice(&result.nodes);
    merged.edges.extend_from_slice(&result.edges);
    merged.hyperedges.extend_from_slice(&result.hyperedges);
    merged.input_tokens += result.input_tokens;
    merged.output_tokens += result.output_tokens;
    merged.elapsed_seconds += result.elapsed_seconds;
}

/// Configuration for [`extract_corpus_parallel`].
pub struct CorpusConfig<'a> {
    /// Backend name (e.g. `"claude"`, `"openai"`).
    pub backend: &'a str,
    /// Optional API key override; falls back to the environment when `None`.
    pub api_key: Option<&'a str>,
    /// Optional model override; uses the backend's default when `None`.
    pub model: Option<&'a str>,
    /// Filesystem root used to compute relative paths in extraction prompts.
    pub root: &'a Path,
    /// Number of files per chunk when `token_budget` is not set.
    pub chunk_size: usize,
    /// Token budget per chunk; when set, `pack_chunks_by_tokens` is used instead of `chunk_size`.
    pub token_budget: Option<usize>,
    /// Maximum number of concurrent extraction workers (Rayon threads).
    pub max_concurrency: usize,
    /// Maximum bisect depth for [`crate::retry::extract_with_adaptive_retry`].
    pub max_retry_depth: usize,
    /// When `true`, use the deep-mode extraction system prompt (`--mode deep`).
    pub deep_mode: bool,
}

/// Callback invoked after each chunk completes successfully.
///
/// Arguments: `(chunk_index, total_chunks, chunk_result)`.
pub type ChunkDoneCb = dyn Fn(usize, usize, &LlmResponse) + Send + Sync;

/// One outcome from processing a single chunk.
enum ChunkOutcome {
    Ok { idx: usize, result: LlmResponse },
    Err { idx: usize, msg: String },
}

/// Extract a corpus in chunks, merging results.
///
/// Uses Rayon for parallelism when `max_concurrency > 1` and the backend
/// allows it (ollama and claude-cli are forced serial by default).
/// A custom `ThreadPool` scoped to this call honours `max_concurrency`.
///
/// Returns the merged response (with `failed_chunk_indices` populated) and a
/// count of failed chunks, matching the Python `extract_corpus_parallel` contract.
///
/// # Panics
/// Never panics in practice; the inner `expect` on a single-thread fallback pool
/// cannot fail because Rayon always permits at least one thread.
#[must_use]
pub fn extract_corpus_parallel(
    files: &[PathBuf],
    cfg: &CorpusConfig<'_>,
    on_chunk_done: Option<&ChunkDoneCb>,
) -> (LlmResponse, usize) {
    let (response, failed, _total) = extract_corpus_parallel_with_total(files, cfg, on_chunk_done);
    (response, failed)
}

/// Same as [`extract_corpus_parallel`] but also returns the total number
/// of chunks attempted, so callers can detect "all chunks failed"
/// (used by the CLI to exit non-zero in that case).
///
/// The returned tuple is `(merged_response, failed_chunk_count, total_chunk_count)`:
/// - `merged_response` is the [`LlmResponse`] containing nodes/edges from every
///   chunk that succeeded.
/// - `failed_chunk_count` is the number of chunks that returned an error.
/// - `total_chunk_count` is the number of chunks attempted overall.
#[must_use]
pub fn extract_corpus_parallel_with_total(
    files: &[PathBuf],
    cfg: &CorpusConfig<'_>,
    on_chunk_done: Option<&ChunkDoneCb>,
) -> (LlmResponse, usize, usize) {
    // Split oversized splittable documents into slices covering the whole file
    // before packing, so content past the char cap is extracted instead of
    // silently dropped (#1369). Files at/under the cap pass through unchanged.
    let units = expand_oversized_files(files, FILE_CHAR_CAP);
    let chunks = pack_chunks(&units, cfg);
    let total = chunks.len();
    let workers = resolve_worker_count(cfg, total);
    let outcomes = run_chunks(&chunks, cfg, workers);
    let (mut response, failed) = merge_outcomes(outcomes, cfg, total, on_chunk_done);
    reconcile_uncovered(&mut response, &chunks, cfg.root);
    (response, failed, total)
}

/// Reconcile dispatched files against those that returned nodes (#1890).
///
/// A semantic chunk can return a clean, non-empty response that omits some of
/// the documents it was given; those docs then vanish from the graph with no
/// node and no warning, and are silently re-dispatched (and re-omitted) forever.
/// Diff the dispatched file set (a slice resolves to its parent file via
/// [`unit_path`], baking in the #cfc7cf2 fix) against the `source_file`s that
/// actually returned, record the gap in `merged.uncovered_files`, and print a
/// loud warning naming the omitted files. Not persisted to `graph.json`.
fn reconcile_uncovered(merged: &mut LlmResponse, chunks: &[Vec<Unit>], root: &Path) {
    use std::collections::{BTreeSet, HashSet};
    // Canonicalize with a fallback to the original path so a missing path never
    // aborts the diff (mirrors Python's `resolve()`-based comparison).
    fn canon(p: &Path) -> PathBuf {
        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
    }
    // Files we dispatched (deduped, sorted). `unit_path` collapses a slice onto
    // its parent file, so a split document counts once.
    let dispatched: BTreeSet<PathBuf> = chunks
        .iter()
        .flatten()
        .map(|u| unit_path(u).to_path_buf())
        .collect();
    // Files that returned: each node's `source_file` resolved against `root`
    // (absolute as-is, else joined), then canonicalized for comparison.
    let covered: HashSet<PathBuf> = merged
        .nodes
        .iter()
        .filter_map(|n| n.get("source_file").and_then(serde_json::Value::as_str))
        .filter(|sf| !sf.is_empty())
        .map(|sf| {
            let p = Path::new(sf);
            let resolved = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            canon(&resolved)
        })
        .collect();
    let uncovered: Vec<String> = dispatched
        .iter()
        .filter(|p| !covered.contains(&canon(p)))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if !uncovered.is_empty() {
        let shown = uncovered
            .iter()
            .take(5)
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>()
            .join(", ");
        let more = if uncovered.len() > 5 {
            format!(" (+{} more)", uncovered.len() - 5)
        } else {
            String::new()
        };
        eprintln!(
            "[graphify] WARNING: {}/{} dispatched file(s) produced no nodes and are \
             absent from the graph: {shown}{more}. The model returned a response but \
             omitted them; a re-run will retry them.",
            uncovered.len(),
            dispatched.len()
        );
    }
    merged.uncovered_files = uncovered;
}

/// Split `files` into chunks using either the token-budget packer or a fixed
/// `chunk_size` slice.
fn pack_chunks(units: &[Unit], cfg: &CorpusConfig<'_>) -> Vec<Vec<Unit>> {
    if let Some(budget) = cfg.token_budget {
        pack_chunks_by_tokens_units(units, budget).unwrap_or_else(|_| {
            units
                .chunks(cfg.chunk_size.max(1))
                .map(<[Unit]>::to_vec)
                .collect()
        })
    } else {
        units
            .chunks(cfg.chunk_size.max(1))
            .map(<[Unit]>::to_vec)
            .collect()
    }
}

/// Decide how many workers to use, honouring backend-specific serial overrides.
fn resolve_worker_count(cfg: &CorpusConfig<'_>, total: usize) -> usize {
    let force_serial = (cfg.backend == "ollama"
        && std::env::var("GRAPHIFY_OLLAMA_PARALLEL")
            .as_deref()
            .unwrap_or("")
            .trim()
            != "1")
        || (cfg.backend == "claude-cli"
            && std::env::var("GRAPHIFY_CLAUDE_CLI_PARALLEL")
                .as_deref()
                .unwrap_or("")
                .trim()
                != "1");
    if force_serial {
        1
    } else {
        cfg.max_concurrency.max(1).min(total.max(1))
    }
}

/// Run a single chunk through the adaptive-retry extractor.
fn extract_one_chunk(idx: usize, chunk: &[Unit], cfg: &CorpusConfig<'_>) -> ChunkOutcome {
    let t0 = Instant::now();
    let ctx = AdaptiveRetryCtx {
        backend: cfg.backend,
        api_key: cfg.api_key,
        model: cfg.model,
        root: cfg.root,
        max_depth: cfg.max_retry_depth,
        deep_mode: cfg.deep_mode,
    };
    match extract_with_adaptive_retry_units(chunk, &ctx, 0) {
        Ok(mut result) => {
            result.elapsed_seconds = t0.elapsed().as_secs_f64();
            ChunkOutcome::Ok { idx, result }
        }
        Err(e) => ChunkOutcome::Err {
            idx,
            msg: e.to_string(),
        },
    }
}

/// Execute all chunks, dispatching to Rayon when `workers > 1`.
fn run_chunks(chunks: &[Vec<Unit>], cfg: &CorpusConfig<'_>, workers: usize) -> Vec<ChunkOutcome> {
    if workers == 1 {
        return chunks
            .iter()
            .enumerate()
            .map(|(idx, chunk)| extract_one_chunk(idx, chunk, cfg))
            .collect();
    }
    // Build a scoped Rayon thread-pool to honour max_concurrency.
    // If pool construction fails fall back to a single-thread pool.
    #[allow(clippy::expect_used)]
    // reason: build() only fails on invalid thread counts which we clamp above.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap_or_else(|_| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .expect("fallback single-thread pool always succeeds")
        });
    pool.install(|| {
        use rayon::prelude::*;
        chunks
            .par_iter()
            .enumerate()
            .map(|(idx, chunk)| extract_one_chunk(idx, chunk, cfg))
            .collect()
    })
}

/// Merge per-chunk outcomes in deterministic order, dispatching `on_chunk_done`.
fn merge_outcomes(
    mut outcomes: Vec<ChunkOutcome>,
    cfg: &CorpusConfig<'_>,
    total: usize,
    on_chunk_done: Option<&ChunkDoneCb>,
) -> (LlmResponse, usize) {
    outcomes.sort_by_key(|o| match o {
        ChunkOutcome::Ok { idx, .. } | ChunkOutcome::Err { idx, .. } => *idx,
    });
    let mut merged = LlmResponse {
        nodes: vec![],
        edges: vec![],
        hyperedges: vec![],
        input_tokens: 0,
        output_tokens: 0,
        model: cfg.model.unwrap_or("").to_string(),
        finish_reason: "stop".to_string(),
        elapsed_seconds: 0.0,
        failed_chunk_indices: vec![],
        uncovered_files: vec![],
    };
    for outcome in outcomes {
        match outcome {
            ChunkOutcome::Ok { idx, result } => {
                merge_into(&mut merged, &result);
                if let Some(cb) = on_chunk_done {
                    cb(idx, total, &result);
                }
            }
            ChunkOutcome::Err { idx, msg } => {
                eprintln!("[graphify] chunk {}/{total} failed: {msg}", idx + 1);
                merged.failed_chunk_indices.push(idx);
            }
        }
    }
    let failed_chunks = merged.failed_chunk_indices.len();
    if failed_chunks > 0 {
        eprintln!(
            "[graphify] WARNING: {failed_chunks}/{total} semantic chunk(s) failed \
             — see errors above. Partial results returned."
        );
    }
    (merged, failed_chunks)
}
