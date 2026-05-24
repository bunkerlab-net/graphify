//! Corpus-parallel extraction with chunk merging.
//!
//! Extracted from `lib.rs` to isolate `extract_corpus_parallel` and
//! `merge_into` — the Rayon-backed fan-out that processes multiple file
//! chunks concurrently and assembles a single merged `LlmResponse`.

use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use crate::LlmResponse;
use crate::retry::extract_with_adaptive_retry;
use crate::tokens::pack_chunks_by_tokens;

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
#[must_use]
pub fn extract_corpus_parallel_with_total(
    files: &[PathBuf],
    cfg: &CorpusConfig<'_>,
    on_chunk_done: Option<&ChunkDoneCb>,
) -> (LlmResponse, usize, usize) {
    let chunks = pack_chunks(files, cfg);
    let total = chunks.len();
    let workers = resolve_worker_count(cfg, total);
    let outcomes = run_chunks(&chunks, cfg, workers);
    let (response, failed) = merge_outcomes(outcomes, cfg, total, on_chunk_done);
    (response, failed, total)
}

/// Split `files` into chunks using either the token-budget packer or a fixed
/// `chunk_size` slice.
fn pack_chunks(files: &[PathBuf], cfg: &CorpusConfig<'_>) -> Vec<Vec<PathBuf>> {
    if let Some(budget) = cfg.token_budget {
        pack_chunks_by_tokens(files, budget).unwrap_or_else(|_| {
            files
                .chunks(cfg.chunk_size.max(1))
                .map(<[PathBuf]>::to_vec)
                .collect()
        })
    } else {
        files
            .chunks(cfg.chunk_size.max(1))
            .map(<[PathBuf]>::to_vec)
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
fn extract_one_chunk(idx: usize, chunk: &[PathBuf], cfg: &CorpusConfig<'_>) -> ChunkOutcome {
    let t0 = Instant::now();
    match extract_with_adaptive_retry(
        chunk,
        cfg.backend,
        cfg.api_key,
        cfg.model,
        cfg.root,
        cfg.max_retry_depth,
        0,
    ) {
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
fn run_chunks(
    chunks: &[Vec<PathBuf>],
    cfg: &CorpusConfig<'_>,
    workers: usize,
) -> Vec<ChunkOutcome> {
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
