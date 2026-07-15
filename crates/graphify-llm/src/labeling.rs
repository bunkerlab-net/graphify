//! LLM-backed community labeling (#1097).
//!
//! Mirrors the `label_communities` / `generate_community_labels` helpers folded
//! into `graphify-py/graphify/llm.py`. When graphify runs inside an
//! orchestrating agent, the agent names communities itself (skill.md Step 5).
//! When run as a bare CLI there is no agent, so these helpers ask the configured
//! backend to name communities in batches of [`LABEL_BATCH_SIZE`] (so each
//! prompt fits a 16k-token context window).
//!
//! Unlike the Python original, the graph is not passed in directly — a
//! `graphify_build::Graph` would couple this low-level crate to the graph model.
//! Callers pass the data the prompt needs: a `node_id → label` map and the set
//! of god-node ids (used only to bias which member labels are sampled first).

use indexmap::{IndexMap, IndexSet};
use rayon::prelude::*;
use regex::Regex;

use crate::LlmError;
use crate::backends::detect_backend;
use crate::call::{UsageSink, call_llm_with_model, call_llm_with_model_usage};
use crate::openai_compat::resolve_max_tokens;

/// Legacy soft-cap on LLM-named communities; kept for callers that want to pin
/// it via [`LabelOptions::max_communities`]. `None` (the default) labels every
/// community.
pub const LABEL_MAX_COMMUNITIES: usize = 200;
/// Node labels sampled per community for the prompt.
const LABEL_TOP_K: usize = 12;
/// Individual labels are truncated to this many chars to keep the prompt small.
const LABEL_MAXLEN: usize = 60;
/// Communities per LLM call; sized for ~16k-token context windows. Splitting
/// into batches keeps self-hosted 16k models (Qwen3, Llama 3.1 8B) from
/// overflowing context and dropping the whole labeling pass to placeholders.
pub const LABEL_BATCH_SIZE: usize = 100;

/// Max recursion depth for [`label_batch_with_retry`]'s split-and-retry on a
/// parse failure, bounding cost. Mirrors Python `_label_batch_with_retry`.
const LABEL_MAX_DEPTH: usize = 3;

/// `true` when an env var opts a serial backend into parallel labeling (value
/// trimmed to exactly `"1"`). Mirrors the Python `GRAPHIFY_*_PARALLEL` switches.
fn env_parallel_opt_in(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| v.trim() == "1")
}

/// `(batch_index, parsed-or-error)` produced by one labeling batch — collected
/// from the worker pool, then merged in index order.
type BatchOutcome = (usize, Result<IndexMap<i64, String>, LlmError>);

/// Knobs for [`label_communities`] / [`label_communities_with`].
///
/// Mirrors the keyword arguments of Python's `label_communities`: `model`,
/// `max_communities`, `top_k`, and `batch_size`. [`LabelOptions::default`]
/// reproduces the Python defaults (label every community in batches of
/// [`LABEL_BATCH_SIZE`], no model override).
#[derive(Clone, Copy, Debug)]
pub struct LabelOptions<'a> {
    /// Optional model override forwarded to the backend (`None` = backend default).
    pub model: Option<&'a str>,
    /// Cap on the total number of communities labeled; `None` labels all.
    pub max_communities: Option<usize>,
    /// Node labels sampled per community for the prompt.
    pub top_k: usize,
    /// Communities per LLM call.
    pub batch_size: usize,
    /// Max batches labeled concurrently. Backends that serialise per process
    /// (ollama, claude-cli) are pinned to 1 unless opted in via env (#1390).
    pub max_concurrency: usize,
}

impl Default for LabelOptions<'_> {
    fn default() -> Self {
        Self {
            model: None,
            max_communities: None,
            top_k: LABEL_TOP_K,
            batch_size: LABEL_BATCH_SIZE,
            max_concurrency: 4,
        }
    }
}

/// Leading/trailing markdown code-fence stripper. Known-good literal pattern.
#[allow(clippy::expect_used)]
static LABEL_FENCE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)^\s*```(?:json)?\s*|\s*```\s*$").expect("static fence regex")
});

/// Salvage regex for complete `"<cid>": "<name>"` pairs from a reply that failed
/// strict JSON parsing (#1690). Known-good literal pattern.
#[allow(clippy::expect_used)]
static LABEL_PAIR_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#""?(-?\d+)"?\s*:\s*"([^"\\]*(?:\\.[^"\\]*)*)""#).expect("static label-pair regex")
});

/// `{cid: "Community {cid}"}` for every community.
#[must_use]
pub fn placeholder_community_labels(
    communities: &IndexMap<i64, Vec<String>>,
) -> IndexMap<i64, String> {
    communities
        .keys()
        .map(|cid| (*cid, format!("Community {cid}")))
        .collect()
}

/// One prompt line per community (largest first), sampling up to `top_k`
/// representative node labels (god nodes first). Returns `(lines, labeled_cids)`;
/// communities with no resolvable labels are skipped.
fn community_label_lines(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    max_communities: usize,
    top_k: usize,
) -> (Vec<String>, Vec<i64>) {
    // Largest community first; stable so ties keep insertion order.
    let mut ordered: Vec<(&i64, &Vec<String>)> = communities.iter().collect();
    ordered.sort_by_key(|(_, members)| std::cmp::Reverse(members.len()));

    let mut lines = Vec::new();
    let mut labeled_cids = Vec::new();
    for (cid, members) in ordered.into_iter().take(max_communities) {
        // God members first, then the rest (stable within each group).
        let ranked = members
            .iter()
            .filter(|m| gods.contains(m.as_str()))
            .chain(members.iter().filter(|m| !gods.contains(m.as_str())));

        let mut names: Vec<String> = Vec::new();
        // Membership-only dedup: `names` carries the observable order, so the
        // set never needs insertion-order tracking.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for nid in ranked {
            let raw = node_labels.get(nid).map_or(nid.as_str(), String::as_str);
            let label: String = raw
                .trim()
                .trim_matches(|c| c == '(' || c == ')')
                .chars()
                .take(LABEL_MAXLEN)
                .collect();
            if !label.is_empty() && seen.insert(label.to_lowercase()) {
                names.push(label);
            }
            if names.len() >= top_k {
                break;
            }
        }
        if !names.is_empty() {
            lines.push(format!("Community {cid}: {}", names.join(", ")));
            labeled_cids.push(*cid);
        }
    }
    (lines, labeled_cids)
}

/// Parse the backend's JSON `{cid: name}` reply. Errors on non-JSON or a
/// non-object payload; silently ignores cids it didn't name.
fn parse_label_response(
    text: &str,
    labeled_cids: &[i64],
) -> Result<IndexMap<i64, String>, LlmError> {
    let cleaned = LABEL_FENCE_RE.replace_all(text.trim(), "").to_string();
    // Always slice the first `{` … last `}` span, even when the reply already
    // starts with `{`, so trailing prose (`{"0":"x"} hope that helps`) is
    // dropped rather than failing the strict parse. Diverges from graphify-py
    // (`llm.py:1308`), which only slices when the text does NOT start with `{`
    // and therefore degrades such replies to placeholders.
    let cleaned = match (cleaned.find('{'), cleaned.rfind('}')) {
        (Some(start), Some(end)) if end > start => cleaned[start..=end].to_string(),
        _ => cleaned,
    };

    let data: serde_json::Value = match serde_json::from_str(&cleaned) {
        Ok(v) => v,
        Err(strict_err) => {
            // #1690: a strict parse failure (e.g. a reply truncated mid-object)
            // is salvaged for its complete `"cid": "name"` pairs rather than
            // losing the whole batch; raise only when nothing recovers.
            let salvaged = salvage_label_pairs(&cleaned, labeled_cids);
            if salvaged.is_empty() {
                return Err(LlmError::Parse(strict_err.to_string()));
            }
            return Ok(salvaged);
        }
    };
    let Some(obj) = data.as_object() else {
        return Err(LlmError::Parse(
            "label response is not a JSON object".to_string(),
        ));
    };

    let mut out = IndexMap::new();
    for &cid in labeled_cids {
        if let Some(name) = obj
            .get(&cid.to_string())
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.insert(cid, name.to_string());
        }
    }
    Ok(out)
}

/// Salvage complete `"<cid>": "<name>"` pairs (restricted to `labeled_cids`)
/// from a reply that failed strict JSON parsing (#1690). Later duplicates win,
/// mirroring Python's dict comprehension.
fn salvage_label_pairs(text: &str, labeled_cids: &[i64]) -> IndexMap<i64, String> {
    let wanted: std::collections::HashSet<i64> = labeled_cids.iter().copied().collect();
    let mut out: IndexMap<i64, String> = IndexMap::new();
    for cap in LABEL_PAIR_RE.captures_iter(text) {
        let (Some(cid_m), Some(name_m)) = (cap.get(1), cap.get(2)) else {
            continue;
        };
        let Ok(cid) = cid_m.as_str().parse::<i64>() else {
            continue;
        };
        if !wanted.contains(&cid) {
            continue;
        }
        let name = name_m.as_str().trim();
        if !name.is_empty() {
            out.insert(cid, name.to_string());
        }
    }
    out
}

/// Label one batch of communities, splitting in half and retrying on a JSON
/// parse failure (#1278).
///
/// Mirrors `_extract_with_adaptive_retry`'s recovery shape for the labeling
/// path: a malformed or non-object reply for a multi-community batch is retried
/// on each half (smaller prompts → less likely to truncate/mangle). At the base
/// case (a single community or `depth >= LABEL_MAX_DEPTH`) the parse error is
/// re-raised so the caller skips that batch. Any non-parse error (network,
/// missing config) propagates unchanged — those are never split-retried.
fn label_batch_with_retry<F>(
    batch_cids: &[i64],
    batch_lines: &[String],
    backend: &str,
    model: Option<&str>,
    depth: usize,
    call: &F,
) -> Result<IndexMap<i64, String>, LlmError>
where
    F: Fn(&str, &str, u32, Option<&str>) -> Result<String, LlmError>,
{
    let prompt = format!(
        "You are naming clusters in a knowledge graph. For each community below, \
         return a concise 2-5 word plain-language name describing what it is about \
         (e.g. \"Order Management\", \"Payment Flow\", \"Auth Middleware\"). \
         Respond ONLY with a JSON object mapping the community id (as a string) to \
         its name - no prose, no markdown fences.\n\n{}",
        batch_lines.join("\n")
    );
    // #1690/#1694: 48 tok/community (was 24) + 256-tok preamble headroom (was 64)
    // so a model that prepends a short preamble can still finish the JSON. Cap at
    // 8192 for 16k-context models; honour GRAPHIFY_MAX_OUTPUT_TOKENS.
    let default_tokens = 256usize
        .saturating_add(48usize.saturating_mul(batch_cids.len()))
        .min(8192);
    let max_tokens = resolve_max_tokens(u32::try_from(default_tokens).unwrap_or(8192));

    match call(&prompt, backend, max_tokens, model)
        .and_then(|text| parse_label_response(&text, batch_cids))
    {
        Ok(parsed) => Ok(parsed),
        // Only a parse failure is recoverable by splitting; anything else
        // (network, missing key) propagates so the caller records it without
        // burning extra calls — matching Python's `(JSONDecodeError, ValueError)`.
        Err(exc) if matches!(exc, LlmError::Parse(_)) => {
            if batch_cids.len() <= 1 || depth >= LABEL_MAX_DEPTH {
                let preview: Vec<i64> = batch_cids.iter().take(5).copied().collect();
                eprintln!(
                    "[graphify label] batch of {} still unparseable at depth {depth} \
                     (cids={preview:?}{}): {exc}",
                    batch_cids.len(),
                    if batch_cids.len() > 5 { "..." } else { "" },
                );
                return Err(exc);
            }
            let mid = batch_cids.len() / 2;
            let mut left = label_batch_with_retry(
                &batch_cids[..mid],
                &batch_lines[..mid],
                backend,
                model,
                depth + 1,
                call,
            )?;
            let right = label_batch_with_retry(
                &batch_cids[mid..],
                &batch_lines[mid..],
                backend,
                model,
                depth + 1,
                call,
            )?;
            left.extend(right);
            Ok(left)
        }
        Err(exc) => Err(exc),
    }
}

/// Return a complete `{cid: name}` map using `backend` for naming.
///
/// Communities are labeled in batches of [`LabelOptions::batch_size`] so each
/// prompt fits a 16k-token context window. Placeholders (`Community N`) are used
/// for any community the backend did not name. Per-batch failures are logged to
/// stderr and skipped — surviving batches still contribute labels. Errors only
/// when *every* batch fails (leaving no labels) so callers wanting graceful
/// degradation can use [`generate_community_labels`].
///
/// # Errors
/// Propagates the first batch's [`LlmError`] when no batch produced any labels.
pub fn label_communities(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: &str,
    opts: LabelOptions<'_>,
) -> Result<IndexMap<i64, String>, LlmError> {
    label_communities_with(
        communities,
        node_labels,
        gods,
        backend,
        opts,
        |prompt, b, max, model| call_llm_with_model(prompt, b, max as usize, model),
    )
}

/// [`label_communities`] with an injectable LLM call — `call(prompt, backend,
/// max_tokens, model)`. Useful for testing without the network, or to route the
/// call through a custom client.
///
/// # Errors
/// Propagates the first batch's error when no batch produced any labels.
pub fn label_communities_with<F>(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: &str,
    opts: LabelOptions<'_>,
    call: F,
) -> Result<IndexMap<i64, String>, LlmError>
where
    F: Fn(&str, &str, u32, Option<&str>) -> Result<String, LlmError> + Sync,
{
    let mut labels = placeholder_community_labels(communities);
    let cap = opts.max_communities.unwrap_or_else(|| communities.len());
    let (lines, labeled_cids) =
        community_label_lines(communities, node_labels, gods, cap, opts.top_k);
    if lines.is_empty() {
        return Ok(labels);
    }

    // `lines` and `labeled_cids` are parallel: line[i] describes labeled_cids[i].
    let batch_size = opts.batch_size.max(1);
    let total = labeled_cids.len();
    let n_batches = total.div_ceil(batch_size);

    // Backends that serialise per process must not fan out: ollama serves one
    // request at a time per loaded model (parallel batches cause VRAM pressure
    // and hollow replies) and claude-cli shells out to a single session that
    // parallel subprocesses corrupt. Force serial unless opted in (#1390).
    let mut max_concurrency = opts.max_concurrency;
    if backend == "ollama" && !env_parallel_opt_in("GRAPHIFY_OLLAMA_PARALLEL") {
        max_concurrency = 1;
    }
    if backend == "claude-cli" && !env_parallel_opt_in("GRAPHIFY_CLAUDE_CLI_PARALLEL") {
        max_concurrency = 1;
    }
    let workers = max_concurrency.min(n_batches).max(1);

    let run_batch = |batch_idx: usize| -> (usize, Result<IndexMap<i64, String>, LlmError>) {
        let start = batch_idx * batch_size;
        let end = (start + batch_size).min(total);
        let parsed = label_batch_with_retry(
            &labeled_cids[start..end],
            &lines[start..end],
            backend,
            opts.model,
            0,
            &call,
        );
        (batch_idx, parsed)
    };

    // Fan out batches across `workers` threads; merge on this thread so `labels`
    // is never mutated concurrently. `workers == 1` keeps the sequential path.
    let mut results: Vec<BatchOutcome> = if workers <= 1 {
        (0..n_batches).map(&run_batch).collect()
    } else {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_or_else(
                |_| (0..n_batches).map(&run_batch).collect(),
                |pool| pool.install(|| (0..n_batches).into_par_iter().map(&run_batch).collect()),
            )
    };
    // Merge in batch order so the propagated error and stderr are deterministic.
    results.sort_by_key(|(batch_idx, _)| *batch_idx);

    let mut written = 0usize;
    let mut first_error: Option<LlmError> = None;
    for (batch_idx, parsed) in results {
        match parsed {
            Ok(parsed) => {
                written += parsed.len();
                labels.extend(parsed);
            }
            Err(exc) => {
                let start = batch_idx * batch_size;
                let end = (start + batch_size).min(total);
                eprintln!(
                    "[graphify label] batch {}/{n_batches} ({} communities) failed: {exc}",
                    batch_idx + 1,
                    end - start,
                );
                if first_error.is_none() {
                    first_error = Some(exc);
                }
            }
        }
    }

    // Every batch failed and produced nothing: propagate so
    // generate_community_labels degrades cleanly to placeholders.
    if written == 0
        && let Some(err) = first_error
    {
        return Err(err);
    }
    Ok(labels)
}

/// Token usage accumulated while naming communities (#1694). Backends that do
/// not report usage (e.g. the Claude Code CLI without a usage envelope)
/// contribute nothing, so the totals are honest rather than estimated.
#[derive(Debug, Clone, Copy, Default)]
pub struct LabelUsage {
    /// Prompt (input) tokens across every labeling call.
    pub input: u64,
    /// Completion (output) tokens across every labeling call.
    pub output: u64,
}

/// CLI entry point: resolve a backend, name communities, and degrade to
/// `Community N` placeholders on any failure (no backend, API error, malformed
/// reply). `model` overrides the backend's default model (`--model`, #b304331).
/// Returns `(labels, source, usage)` where `source` is `"llm"` or
/// `"placeholder"` and `usage` totals the labeling token cost (#1694).
/// Never errors.
#[must_use]
// Labeling entry point: graph data + backend auto-detect + tuning knobs; a
// partial options-struct split would obscure the auto-detect/degrade flow.
#[allow(clippy::too_many_arguments)]
pub fn generate_community_labels(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: Option<&str>,
    model: Option<&str>,
    quiet: bool,
    max_concurrency: usize,
    batch_size: usize,
) -> (IndexMap<i64, String>, &'static str, LabelUsage) {
    // Accumulate token usage from the real backend calls so cluster-only mode
    // can report the labeling cost (#1694). The sink is `Sync`, so the rayon
    // fan-out inside `label_communities_with` records into it concurrently.
    let usage = UsageSink::new();
    let (labels, source) = generate_community_labels_with(
        communities,
        node_labels,
        gods,
        backend,
        model,
        quiet,
        max_concurrency,
        batch_size,
        |prompt, b, max, m| call_llm_with_model_usage(prompt, b, max as usize, m, Some(&usage)),
    );
    (
        labels,
        source,
        LabelUsage {
            input: usage.input(),
            output: usage.output(),
        },
    )
}

/// [`generate_community_labels`] with an injectable LLM call — `call(prompt,
/// backend, max_tokens, model)`. Used by the public wrapper and by tests.
#[must_use]
// As `generate_community_labels`, plus the injectable `call` for testing.
#[allow(clippy::too_many_arguments)]
pub fn generate_community_labels_with<F>(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: Option<&str>,
    model: Option<&str>,
    quiet: bool,
    max_concurrency: usize,
    batch_size: usize,
    call: F,
) -> (IndexMap<i64, String>, &'static str)
where
    F: Fn(&str, &str, u32, Option<&str>) -> Result<String, LlmError> + Sync,
{
    let resolved = match backend {
        Some(b) if !b.is_empty() => Some(b.to_string()),
        _ => detect_backend(),
    };
    let Some(backend) = resolved else {
        if !quiet {
            eprintln!(
                "[graphify label] no LLM backend configured; keeping Community N \
                 placeholders. Set an API key (e.g. GOOGLE_API_KEY) or pass --backend."
            );
        }
        return (placeholder_community_labels(communities), "placeholder");
    };
    let opts = LabelOptions {
        model,
        batch_size,
        max_concurrency,
        ..LabelOptions::default()
    };
    match label_communities_with(communities, node_labels, gods, &backend, opts, call) {
        Ok(labels) => (labels, "llm"),
        Err(exc) => {
            if !quiet {
                eprintln!(
                    "[graphify label] warning: community labeling failed ({exc}); \
                     using Community N placeholders."
                );
            }
            (placeholder_community_labels(communities), "placeholder")
        }
    }
}
