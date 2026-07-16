//! Adaptive retry with context-overflow detection.
//!
//! Extracted from `lib.rs` to isolate `extract_with_adaptive_retry` and
//! `looks_like_context_exceeded` — the halving-retry loop that splits
//! oversized chunks rather than surfacing context-window errors to callers.

use std::path::{Path, PathBuf};

use crate::extract::extract_units_direct_mode;
use crate::file_slice::{FileSlice, Unit, bisect_slice, unit_path};
use crate::{LlmError, LlmResponse};

/// Run-constant context for [`extract_with_adaptive_retry_ctx`].
///
/// Bundles the fields that stay fixed across the bisect recursion so the
/// recursive entry point stays a 3-argument call (`chunk`, `ctx`, `depth`).
pub(crate) struct AdaptiveRetryCtx<'a> {
    pub backend: &'a str,
    pub api_key: Option<&'a str>,
    pub model: Option<&'a str>,
    pub root: &'a std::path::Path,
    pub max_depth: usize,
    pub deep_mode: bool,
}

const CONTEXT_EXCEEDED_MARKERS: &[&str] = &[
    "context size",
    "context length",
    "context_length",
    "context window",
    "n_keep",
    "exceeds the available",
    "n_ctx",
    "maximum context",
    "too many tokens",
    "prompt is too long",
    "context_length_exceeded",
];

/// Heuristically classify an error as context-window overflow.
#[must_use]
pub fn looks_like_context_exceeded(err: &LlmError) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    CONTEXT_EXCEEDED_MARKERS.iter().any(|m| msg.contains(m))
}

/// Same check against a boxed `std::error::Error`.
#[must_use]
pub fn looks_like_context_exceeded_dyn(err: &(dyn std::error::Error + Send + Sync)) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    CONTEXT_EXCEEDED_MARKERS.iter().any(|m| msg.contains(m))
}

/// Construct a zero-filled [`LlmResponse`] for use as the identity element when merging.
///
/// Used as the base accumulator in `merge_responses` and as the fallback
/// result when a chunk fails after exhausting all retry attempts.
pub(crate) fn empty_llm_response(model: Option<&str>) -> LlmResponse {
    LlmResponse {
        nodes: vec![],
        edges: vec![],
        hyperedges: vec![],
        input_tokens: 0,
        output_tokens: 0,
        model: model.unwrap_or("").to_string(),
        finish_reason: "stop".to_string(),
        elapsed_seconds: 0.0,
        failed_chunk_indices: vec![],
        uncovered_files: vec![],
    }
}

/// Combine two [`LlmResponse`] values into one by concatenating their node/edge
/// lists and summing token counts.
///
/// Used by `extract_with_adaptive_retry` to reunite the two halves after a
/// successful bisect-and-retry.
pub(crate) fn merge_responses(
    left: &LlmResponse,
    right: &LlmResponse,
    model: Option<&str>,
) -> LlmResponse {
    let mut nodes = left.nodes.clone();
    nodes.extend_from_slice(&right.nodes);
    let mut edges = left.edges.clone();
    edges.extend_from_slice(&right.edges);
    let mut hyperedges = left.hyperedges.clone();
    hyperedges.extend_from_slice(&right.hyperedges);
    let mut failed_chunk_indices = left.failed_chunk_indices.clone();
    failed_chunk_indices.extend_from_slice(&right.failed_chunk_indices);
    let mut uncovered_files = left.uncovered_files.clone();
    uncovered_files.extend_from_slice(&right.uncovered_files);
    LlmResponse {
        nodes,
        edges,
        hyperedges,
        input_tokens: left.input_tokens + right.input_tokens,
        output_tokens: left.output_tokens + right.output_tokens,
        model: model.map_or_else(|| left.model.clone(), str::to_string),
        finish_reason: "stop".to_string(),
        // Merge: sum elapsed, concatenate failed indices from both halves.
        elapsed_seconds: left.elapsed_seconds + right.elapsed_seconds,
        failed_chunk_indices,
        uncovered_files,
    }
}

/// Extract a chunk; split in half and retry on context overflow or truncation.
///
/// Standard (non-deep) entry point. See [`extract_with_adaptive_retry_units`] to
/// thread deep mode, run-constant settings, and `FileSlice` units.
///
/// # Errors
/// Propagates errors that don't look like context-window overflow.
pub fn extract_with_adaptive_retry(
    chunk: &[PathBuf],
    backend: &str,
    api_key: Option<&str>,
    model: Option<&str>,
    root: &Path,
    max_depth: usize,
    depth: usize,
) -> Result<LlmResponse, LlmError> {
    let ctx = AdaptiveRetryCtx {
        backend,
        api_key,
        model,
        root,
        max_depth,
        deep_mode: false,
    };
    let units: Vec<Unit> = chunk.iter().map(|p| Unit::Whole(p.clone())).collect();
    extract_with_adaptive_retry_units(&units, &ctx, depth)
}

/// Extract a chunk of [`Unit`]s; split in half and retry on context overflow or
/// truncation, honouring the run-constant settings in `ctx` (including
/// `deep_mode`).
///
/// A single-unit chunk that overflows or truncates is recoverable only when it
/// is a [`Unit::Slice`] of a splittable document: the slice is bisected and
/// retried (#1369). A whole non-splittable file can't be made smaller than
/// itself, so the (partial) result is surfaced with a warning.
///
/// # Errors
/// Propagates errors that don't look like context-window overflow.
pub(crate) fn extract_with_adaptive_retry_units(
    chunk: &[Unit],
    ctx: &AdaptiveRetryCtx<'_>,
    depth: usize,
) -> Result<LlmResponse, LlmError> {
    let result = extract_units_direct_mode(
        chunk,
        ctx.backend,
        ctx.api_key,
        ctx.model,
        ctx.root,
        ctx.deep_mode,
    );

    match result {
        Err(ref e) if looks_like_context_exceeded(e) => {
            if chunk.len() <= 1 {
                if let Some((left, right)) = split_lone_slice(chunk, ctx, depth) {
                    eprintln!(
                        "[graphify] slice of {} exceeded context at depth {depth}; \
                         splitting the slice and retrying",
                        unit_path(&chunk[0]).display()
                    );
                    return merge_two(&[Unit::Slice(left)], &[Unit::Slice(right)], ctx, depth);
                }
                eprintln!(
                    "[graphify] single-file chunk {} exceeds model context \
                     and cannot be split further: {e}",
                    chunk
                        .first()
                        .map(|u| unit_path(u).display().to_string())
                        .unwrap_or_default()
                );
                return Ok(empty_llm_response(ctx.model));
            }
            if depth >= ctx.max_depth {
                eprintln!(
                    "[graphify] chunk of {} still overflows context at recursion \
                     depth {depth} (max {}) — dropping",
                    chunk.len(),
                    ctx.max_depth
                );
                return Ok(empty_llm_response(ctx.model));
            }
            eprintln!(
                "[graphify] chunk of {} exceeded context at depth {depth} \
                 (context overflow); splitting in half and retrying",
                chunk.len()
            );
            let mid = chunk.len() / 2;
            merge_two(&chunk[..mid], &chunk[mid..], ctx, depth)
        }
        Err(e) => Err(e),
        Ok(resp) if resp.finish_reason == "length" => {
            if chunk.len() <= 1 {
                if let Some((left, right)) = split_lone_slice(chunk, ctx, depth) {
                    eprintln!(
                        "[graphify] slice of {} truncated at depth {depth}; \
                         splitting the slice and retrying",
                        unit_path(&chunk[0]).display()
                    );
                    return merge_two(&[Unit::Slice(left)], &[Unit::Slice(right)], ctx, depth);
                }
                eprintln!(
                    "[graphify] single-file chunk {} truncated at \
                     max_completion_tokens — partial result kept",
                    chunk
                        .first()
                        .map(|u| unit_path(u).display().to_string())
                        .unwrap_or_default()
                );
                return Ok(resp);
            }
            if depth >= ctx.max_depth {
                eprintln!(
                    "[graphify] chunk of {} still truncated at recursion depth {depth} \
                     (max {}) — partial result kept",
                    chunk.len(),
                    ctx.max_depth
                );
                return Ok(resp);
            }
            eprintln!(
                "[graphify] chunk of {} truncated at depth {depth}, \
                 splitting into halves of {} and {}",
                chunk.len(),
                chunk.len() / 2,
                chunk.len() - chunk.len() / 2,
            );
            let mid = chunk.len() / 2;
            merge_two(&chunk[..mid], &chunk[mid..], ctx, depth)
        }
        Ok(resp) => Ok(resp),
    }
}

/// Recurse on both halves at `depth + 1` and reunite the results.
fn merge_two(
    left: &[Unit],
    right: &[Unit],
    ctx: &AdaptiveRetryCtx<'_>,
    depth: usize,
) -> Result<LlmResponse, LlmError> {
    let l = extract_with_adaptive_retry_units(left, ctx, depth + 1)?;
    let r = extract_with_adaptive_retry_units(right, ctx, depth + 1)?;
    Ok(merge_responses(&l, &r, ctx.model))
}

/// Bisect a single-unit chunk when it is a slice and depth allows, so an
/// oversized slice is retried on a smaller range rather than dropped (#1369).
fn split_lone_slice(
    chunk: &[Unit],
    ctx: &AdaptiveRetryCtx<'_>,
    depth: usize,
) -> Option<(FileSlice, FileSlice)> {
    if chunk.len() == 1
        && depth < ctx.max_depth
        && let Unit::Slice(fs) = &chunk[0]
    {
        return bisect_slice(fs);
    }
    None
}
