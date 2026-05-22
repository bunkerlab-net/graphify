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
    pub backend: &'a str,
    pub api_key: Option<&'a str>,
    pub model: Option<&'a str>,
    pub root: &'a Path,
    pub chunk_size: usize,
    pub token_budget: Option<usize>,
    pub max_concurrency: usize,
    pub max_retry_depth: usize,
}

/// Callback type for chunk-done notifications.
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
#[allow(clippy::too_many_lines)]
pub fn extract_corpus_parallel(
    files: &[PathBuf],
    cfg: &CorpusConfig<'_>,
    on_chunk_done: Option<&ChunkDoneCb>,
) -> (LlmResponse, usize) {
    let chunks: Vec<Vec<PathBuf>> = if let Some(budget) = cfg.token_budget {
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
    };

    let total = chunks.len();

    // Force serial for backends that don't support concurrent calls.
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
    let workers = if force_serial {
        1_usize
    } else {
        cfg.max_concurrency.max(1).min(total.max(1))
    };

    // Run chunks — serial path avoids Rayon overhead and keeps callback ordering
    // identical to the pre-parallelism sequential path (matches Python's serial branch).
    let outcomes: Vec<ChunkOutcome> = if workers == 1 {
        chunks
            .iter()
            .enumerate()
            .map(|(idx, chunk)| {
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
            })
            .collect()
    } else {
        // Build a scoped Rayon thread-pool to honour max_concurrency.
        // If pool construction fails fall back to the global pool.
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
                .map(|(idx, chunk)| {
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
                })
                .collect()
        })
    };

    // Merge outcomes in chunk-index order so the merged result is deterministic.
    let mut ordered: Vec<ChunkOutcome> = outcomes;
    ordered.sort_by_key(|o| match o {
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

    for outcome in ordered {
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
