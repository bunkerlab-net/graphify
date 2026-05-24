//! Semantic-extraction fragment validation and cleanup.
//!
//! Ports `graphify-py/graphify/semantic_cleanup.py`. Used by the LLM
//! merge scripts (skill-opencode, skill-codex) to keep agent-emitted
//! rationale text out of the knowledge graph and to enforce hard size /
//! shape limits on untrusted JSON before it touches the build pipeline.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

/// Maximum encoded size of a semantic fragment (bytes after UTF-8 encode).
pub const MAX_SEMANTIC_FRAGMENT_BYTES: u64 = 25 * 1024 * 1024;
/// Maximum number of nodes in a single fragment.
pub const MAX_SEMANTIC_FRAGMENT_NODES: usize = 10_000;
/// Maximum number of edges in a single fragment.
pub const MAX_SEMANTIC_FRAGMENT_EDGES: usize = 100_000;
/// Maximum number of hyperedges in a single fragment.
pub const MAX_SEMANTIC_FRAGMENT_HYPEREDGES: usize = 10_000;
/// Maximum number of node IDs referenced by a single hyperedge.
pub const MAX_SEMANTIC_HYPEREDGE_NODES: usize = 256;
/// Maximum character length of any node/edge/hyperedge ID.
pub const MAX_SEMANTIC_ID_LENGTH: usize = 256;

/// Allowed `file_type` values inside a semantic fragment.
///
/// Note: `"rationale"` and `"concept"` are listed here because nodes with
/// those types are *allowed* through the validator, but they are
/// subsequently stripped (or converted to attributes) by
/// [`sanitize_semantic_fragment`].
pub const VALID_SEMANTIC_FILE_TYPES: &[&str] =
    &["code", "document", "paper", "image", "rationale", "concept"];

const RATIONALE_MIN_CHARS: usize = 80;
const RATIONALE_MIN_WORDS: usize = 8;

const REGEX_CHARSET: &str = r"^[A-Za-z0-9._:-]+$";

#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static SEMANTIC_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(REGEX_CHARSET).expect("static semantic-id regex"));

#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static SENTENCE_PUNCT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[.!?:]").expect("static sentence-punct regex"));

/// Validate an untrusted semantic-extraction fragment.
///
/// Returns the list of human-readable error strings (empty when the
/// fragment is valid). Accepts arbitrary [`Value`] rather than a typed
/// shape because the input is untrusted JSON.
#[allow(clippy::too_many_lines)] // schema-walk dispatch — splitting hurts readability
#[must_use]
pub fn validate_semantic_fragment(fragment: &Value) -> Vec<String> {
    let Value::Object(obj) = fragment else {
        return vec!["fragment must be a JSON object".to_string()];
    };

    let mut errors: Vec<String> = Vec::new();

    // Compute the encoded payload size to enforce
    // `MAX_SEMANTIC_FRAGMENT_BYTES`. Serialisation failure must be
    // surfaced as a validation error rather than silently treated as a
    // zero-byte payload (the latter would let a malformed fragment that
    // re-serialises poorly slip through the size cap).
    let payload_len = match serde_json::to_vec(fragment) {
        Ok(b) => b.len(),
        Err(err) => {
            errors.push(format!("failed to compute payload size: {err}"));
            return errors;
        }
    };
    if (payload_len as u64) > MAX_SEMANTIC_FRAGMENT_BYTES {
        errors.push(format!(
            "payload is {payload_len} bytes; max is {MAX_SEMANTIC_FRAGMENT_BYTES}"
        ));
    }

    let nodes_default: Vec<Value> = Vec::new();
    let nodes = match obj.get("nodes") {
        Some(Value::Array(arr)) => {
            if arr.len() > MAX_SEMANTIC_FRAGMENT_NODES {
                errors.push(format!(
                    "nodes has {} entries; max is {MAX_SEMANTIC_FRAGMENT_NODES}",
                    arr.len()
                ));
            }
            arr.clone()
        }
        Some(_) => {
            errors.push("nodes must be a list".to_string());
            nodes_default
        }
        None => nodes_default,
    };

    let edges_default: Vec<Value> = Vec::new();
    let edges = match obj.get("edges") {
        Some(Value::Array(arr)) => {
            if arr.len() > MAX_SEMANTIC_FRAGMENT_EDGES {
                errors.push(format!(
                    "edges has {} entries; max is {MAX_SEMANTIC_FRAGMENT_EDGES}",
                    arr.len()
                ));
            }
            arr.clone()
        }
        Some(_) => {
            errors.push("edges must be a list".to_string());
            edges_default
        }
        None => edges_default,
    };

    for (i, node) in nodes.iter().enumerate() {
        let Value::Object(map) = node else {
            errors.push(format!("nodes[{i}] must be an object"));
            continue;
        };
        validate_semantic_id(&mut errors, &format!("nodes[{i}].id"), map.get("id"));
        if let Some(ft) = map.get("file_type")
            && !ft.is_null()
        {
            let ft_str = ft.as_str().unwrap_or_default();
            if !VALID_SEMANTIC_FILE_TYPES.contains(&ft_str) {
                let mut sorted: Vec<&str> = VALID_SEMANTIC_FILE_TYPES.to_vec();
                sorted.sort_unstable();
                errors.push(format!(
                    "nodes[{i}].file_type {ft:?} is not one of {sorted:?}"
                ));
            }
        }
    }

    for (i, edge) in edges.iter().enumerate() {
        let Value::Object(map) = edge else {
            errors.push(format!("edges[{i}] must be an object"));
            continue;
        };
        validate_semantic_id(
            &mut errors,
            &format!("edges[{i}].source"),
            map.get("source"),
        );
        validate_semantic_id(
            &mut errors,
            &format!("edges[{i}].target"),
            map.get("target"),
        );
    }

    if let Some(hyperedges_val) = obj.get("hyperedges")
        && !hyperedges_val.is_null()
    {
        if let Some(hyperedges) = hyperedges_val.as_array() {
            if hyperedges.len() > MAX_SEMANTIC_FRAGMENT_HYPEREDGES {
                errors.push(format!(
                    "hyperedges has {} entries; max is {MAX_SEMANTIC_FRAGMENT_HYPEREDGES}",
                    hyperedges.len()
                ));
            }
            for (i, he) in hyperedges.iter().enumerate() {
                let Value::Object(map) = he else {
                    errors.push(format!("hyperedges[{i}] must be an object"));
                    continue;
                };
                validate_semantic_id(&mut errors, &format!("hyperedges[{i}].id"), map.get("id"));
                let Some(he_nodes) = map.get("nodes").and_then(Value::as_array) else {
                    errors.push(format!("hyperedges[{i}].nodes must be a list"));
                    continue;
                };
                if he_nodes.len() > MAX_SEMANTIC_HYPEREDGE_NODES {
                    errors.push(format!(
                        "hyperedges[{i}].nodes has {} entries; max is {MAX_SEMANTIC_HYPEREDGE_NODES}",
                        he_nodes.len()
                    ));
                }
                for (j, refv) in he_nodes.iter().enumerate() {
                    validate_semantic_id(
                        &mut errors,
                        &format!("hyperedges[{i}].nodes[{j}]"),
                        Some(refv),
                    );
                }
            }
        } else {
            errors.push("hyperedges must be a list".to_string());
        }
    }

    errors
}

/// Sanitise a fragment in place by mutating the provided [`Map`].
///
/// Runs four discrete cleanup passes:
/// 1. Strip nodes with invalid `file_type` (`"rationale"` / `"concept"`).
/// 2. Convert sentence-like `rationale_for` source nodes into `rationale`
///    attributes on their targets.
/// 3. Drop edges that reference removed nodes.
/// 4. Filter hyperedges to their surviving members (drop the hyperedge
///    entirely when fewer than two members survive).
#[allow(clippy::too_many_lines)] // four discrete cleanup passes — split would obscure their order
pub fn sanitize_semantic_fragment(fragment: &mut Map<String, Value>) {
    let invalid_ft = ["rationale", "concept"];

    let nodes: Vec<Value> = fragment
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edges: Vec<Value> = fragment
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let hyperedges: Vec<Value> = fragment
        .get("hyperedges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Build the ID set for surviving target lookups. Only `contains`
    // is needed downstream, so an IndexSet avoids cloning each entire
    // node value.
    let mut node_ids: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    for n in &nodes {
        let id = n
            .as_object()
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !id.is_empty() {
            node_ids.insert(id);
        }
    }

    // Pre-collect node IDs that source a `rationale_for` edge.
    let mut rationale_for_sources: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    for e in &edges {
        if let Some(map) = e.as_object()
            && map.get("relation").and_then(Value::as_str) == Some("rationale_for")
            && let Some(src) = map.get("source").and_then(Value::as_str)
            && !src.is_empty()
        {
            rationale_for_sources.insert(src.to_string());
        }
    }

    // Pass 1: identify removal/rationale candidates.
    let mut rationale_candidates: Vec<Value> = Vec::new();
    let mut remove_ids: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    let mut keep_nodes: Vec<Value> = Vec::new();
    for n in &nodes {
        let Some(map) = n.as_object() else {
            continue;
        };
        let nid = map.get("id").and_then(Value::as_str).unwrap_or_default();
        if nid.is_empty() {
            continue;
        }
        let ft = map.get("file_type").and_then(Value::as_str).unwrap_or("");
        let label = map.get("label").and_then(Value::as_str).unwrap_or("");
        if invalid_ft.contains(&ft) {
            if is_sentence_like_rationale_label(label) {
                rationale_candidates.push(n.clone());
            }
            remove_ids.insert(nid.to_string());
            continue;
        }
        if rationale_for_sources.contains(nid) && is_sentence_like_rationale_label(label) {
            rationale_candidates.push(n.clone());
            remove_ids.insert(nid.to_string());
            continue;
        }
        keep_nodes.push(n.clone());
    }

    // Pass 2: propagate rationale text via `rationale_for` edges only.
    let mut rationale_attrs: indexmap::IndexMap<String, Vec<String>> = indexmap::IndexMap::new();
    for rn in &rationale_candidates {
        let Some(map) = rn.as_object() else {
            continue;
        };
        let rn_id = map.get("id").and_then(Value::as_str).unwrap_or("");
        let text = map
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        for e in &edges {
            let Some(em) = e.as_object() else {
                continue;
            };
            if em.get("relation").and_then(Value::as_str) != Some("rationale_for") {
                continue;
            }
            if em.get("source").and_then(Value::as_str) != Some(rn_id) {
                continue;
            }
            let Some(target_id) = em.get("target").and_then(Value::as_str) else {
                continue;
            };
            if !node_ids.contains(target_id) || remove_ids.contains(target_id) {
                continue;
            }
            rationale_attrs
                .entry(target_id.to_string())
                .or_default()
                .push(text.clone());
        }
    }

    // Apply rationale-attr propagation: mutate the corresponding entries in
    // keep_nodes (since we already cloned them out of the input list).
    for n in &mut keep_nodes {
        let Some(map) = n.as_object_mut() else {
            continue;
        };
        let Some(nid) = map.get("id").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        if let Some(texts) = rationale_attrs.get(&nid) {
            append_rationale_attr(map, texts);
        }
    }

    // Pass 3: drop edges referencing removed nodes, and drop malformed
    // non-object edges entirely. The previous `return true` for
    // non-objects preserved garbage that would only blow up downstream
    // validation; treating malformed edges as removable is the more
    // honest behaviour.
    let keep_edges: Vec<Value> = edges
        .into_iter()
        .filter(|e| {
            let Some(map) = e.as_object() else {
                return false;
            };
            let src = map.get("source").and_then(Value::as_str).unwrap_or("");
            let tgt = map.get("target").and_then(Value::as_str).unwrap_or("");
            !remove_ids.contains(src) && !remove_ids.contains(tgt)
        })
        .collect();

    // Pass 4: filter hyperedges to surviving members.
    let surviving_ids: indexmap::IndexSet<String> = keep_nodes
        .iter()
        .filter_map(|n| {
            n.as_object()
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    let mut keep_hyperedges: Vec<Value> = Vec::new();
    for he in hyperedges {
        let Some(map) = he.as_object() else {
            continue;
        };
        let Some(he_nodes) = map.get("nodes").and_then(Value::as_array) else {
            continue;
        };
        let original_len = he_nodes.len();
        let filtered: Vec<Value> = he_nodes
            .iter()
            .filter(|v| {
                v.as_str()
                    .is_some_and(|s| surviving_ids.contains(&s.to_string()))
            })
            .cloned()
            .collect();
        if filtered.len() < 2 {
            continue;
        }
        let mut new_map = map.clone();
        if filtered.len() != original_len {
            new_map.insert("nodes".to_string(), Value::Array(filtered));
        }
        keep_hyperedges.push(Value::Object(new_map));
    }

    fragment.insert("nodes".to_string(), Value::Array(keep_nodes));
    fragment.insert("edges".to_string(), Value::Array(keep_edges));
    fragment.insert("hyperedges".to_string(), Value::Array(keep_hyperedges));
}

/// Load a semantic fragment from disk and validate it.
///
/// Returns `(Some(fragment), [])` on success; `(None, errors)` when the
/// file is missing, oversize, malformed, or fails [`validate_semantic_fragment`].
///
/// The size guard runs against the file's `metadata().len()` so a
/// multi-gigabyte chunk file cannot blow up memory during the read.
#[must_use]
pub fn load_validated_semantic_fragment(path: &Path) -> (Option<Value>, Vec<String>) {
    let size = match path.metadata() {
        Ok(m) => m.len(),
        Err(err) => {
            return (
                None,
                vec![format!("could not stat {}: {err}", path.display())],
            );
        }
    };
    if size > MAX_SEMANTIC_FRAGMENT_BYTES {
        return (
            None,
            vec![format!(
                "payload is {size} bytes; max is {MAX_SEMANTIC_FRAGMENT_BYTES}"
            )],
        );
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(err) => {
            return (
                None,
                vec![format!("could not read {}: {err}", path.display())],
            );
        }
    };
    let fragment: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(err) => return (None, vec![format!("invalid JSON: {err}")]),
    };
    let errors = validate_semantic_fragment(&fragment);
    if errors.is_empty() {
        (Some(fragment), Vec::new())
    } else {
        (None, errors)
    }
}

fn validate_semantic_id(errors: &mut Vec<String>, field: &str, value: Option<&Value>) {
    let Some(v) = value else {
        errors.push(format!("{field} must be a string"));
        return;
    };
    let Some(s) = v.as_str() else {
        errors.push(format!("{field} must be a string"));
        return;
    };
    if s.is_empty() {
        errors.push(format!("{field} must not be empty"));
        return;
    }
    if s.chars().count() > MAX_SEMANTIC_ID_LENGTH {
        errors.push(format!(
            "{field} is {} chars; max is {MAX_SEMANTIC_ID_LENGTH}",
            s.chars().count()
        ));
    }
    if s.contains('/') || s.contains('\\') || s.contains("..") {
        errors.push(format!("{field} must not contain path separators or '..'"));
    }
    if !SEMANTIC_ID_RE.is_match(s) {
        errors.push(format!("{field} contains unsupported characters"));
    }
}

fn is_sentence_like_rationale_label(label: &str) -> bool {
    let label = label.trim();
    if label.is_empty() {
        return false;
    }
    let chars_count = label.chars().count();
    if chars_count < RATIONALE_MIN_CHARS {
        let words = label.split_whitespace().count();
        if words < RATIONALE_MIN_WORDS {
            return false;
        }
    }
    SENTENCE_PUNCT.is_match(label)
}

fn append_rationale_attr(node: &mut Map<String, Value>, texts: &[String]) {
    let new_text = texts.join("\n\n").trim().to_string();
    let existing = node
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let merged = if existing.is_empty() {
        new_text
    } else {
        format!("{existing}\n\n{new_text}")
    };
    node.insert("rationale".to_string(), Value::String(merged));
}
