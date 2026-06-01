//! LLM-backed community labeling (#1097).
//!
//! Mirrors the `label_communities` / `generate_community_labels` helpers folded
//! into `graphify-py/graphify/llm.py`. When graphify runs inside an
//! orchestrating agent, the agent names communities itself (skill.md Step 5).
//! When run as a bare CLI there is no agent, so these helpers ask the configured
//! backend to name communities in ONE batched call.
//!
//! Unlike the Python original, the graph is not passed in directly — a
//! `graphify_build::Graph` would couple this low-level crate to the graph model.
//! Callers pass the data the prompt needs: a `node_id → label` map and the set
//! of god-node ids (used only to bias which member labels are sampled first).

use indexmap::{IndexMap, IndexSet};
use regex::Regex;

use crate::LlmError;
use crate::backends::detect_backend;
use crate::call::call_llm;

/// Cap on LLM-named communities; the tail keeps `Community N` placeholders.
const LABEL_MAX_COMMUNITIES: usize = 200;
/// Node labels sampled per community for the prompt.
const LABEL_TOP_K: usize = 12;
/// Individual labels are truncated to this many chars to keep the prompt small.
const LABEL_MAXLEN: usize = 60;

/// Leading/trailing markdown code-fence stripper. Known-good literal pattern.
#[allow(clippy::expect_used)]
static LABEL_FENCE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)^\s*```(?:json)?\s*|\s*```\s*$").expect("static fence regex")
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
        let mut seen: IndexSet<String> = IndexSet::new();
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

    let data: serde_json::Value =
        serde_json::from_str(&cleaned).map_err(|e| LlmError::Parse(e.to_string()))?;
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

/// Return a complete `{cid: name}` map using `backend` for naming.
///
/// Placeholders (`Community N`) are used for any community the backend did not
/// name. Errors on backend/parse failure — callers wanting graceful degradation
/// should use [`generate_community_labels`].
///
/// # Errors
/// Propagates [`LlmError`] from the backend call or a malformed JSON reply.
pub fn label_communities(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: &str,
) -> Result<IndexMap<i64, String>, LlmError> {
    label_communities_with(communities, node_labels, gods, backend, |prompt, b, max| {
        call_llm(prompt, b, max as usize)
    })
}

/// [`label_communities`] with an injectable LLM call — `call(prompt, backend,
/// max_tokens)`. Useful for testing without the network, or to route the call
/// through a custom client.
///
/// # Errors
/// Propagates whatever `call` returns, plus parse errors on a malformed reply.
pub fn label_communities_with<F>(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: &str,
    call: F,
) -> Result<IndexMap<i64, String>, LlmError>
where
    F: Fn(&str, &str, u32) -> Result<String, LlmError>,
{
    let mut labels = placeholder_community_labels(communities);
    let (lines, labeled_cids) = community_label_lines(
        communities,
        node_labels,
        gods,
        LABEL_MAX_COMMUNITIES,
        LABEL_TOP_K,
    );
    if lines.is_empty() {
        return Ok(labels);
    }

    let prompt = format!(
        "You are naming clusters in a knowledge graph. For each community below, \
         return a concise 2-5 word plain-language name describing what it is about \
         (e.g. \"Order Management\", \"Payment Flow\", \"Auth Middleware\"). \
         Respond ONLY with a JSON object mapping the community id (as a string) to \
         its name - no prose, no markdown fences.\n\n{}",
        lines.join("\n")
    );

    let labeled_len = u32::try_from(labeled_cids.len()).unwrap_or(u32::MAX);
    let max_tokens = 16u32
        .saturating_mul(labeled_len)
        .saturating_add(40)
        .min(4096);
    let text = call(&prompt, backend, max_tokens)?;
    labels.extend(parse_label_response(&text, &labeled_cids)?);
    Ok(labels)
}

/// CLI entry point: resolve a backend, name communities, and degrade to
/// `Community N` placeholders on any failure (no backend, API error, malformed
/// reply). Returns `(labels, source)` where `source` is `"llm"` or
/// `"placeholder"`. Never errors.
#[must_use]
pub fn generate_community_labels(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: Option<&str>,
    quiet: bool,
) -> (IndexMap<i64, String>, &'static str) {
    generate_community_labels_with(
        communities,
        node_labels,
        gods,
        backend,
        quiet,
        |prompt, b, max| call_llm(prompt, b, max as usize),
    )
}

/// [`generate_community_labels`] with an injectable LLM call — `call(prompt,
/// backend, max_tokens)`. Used by the public wrapper and by tests.
#[must_use]
pub fn generate_community_labels_with<F>(
    communities: &IndexMap<i64, Vec<String>>,
    node_labels: &IndexMap<String, String>,
    gods: &IndexSet<String>,
    backend: Option<&str>,
    quiet: bool,
    call: F,
) -> (IndexMap<i64, String>, &'static str)
where
    F: Fn(&str, &str, u32) -> Result<String, LlmError>,
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
    match label_communities_with(communities, node_labels, gods, &backend, call) {
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
