//! Label-canonical node deduplication. Ports `build._norm_label` +
//! `deduplicate_by_label`.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

#[allow(clippy::expect_used)] // literal pattern; build cannot panic.
static NON_ALNUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9 ]").expect("static non-alnum regex"));

#[allow(clippy::expect_used)] // literal pattern; build cannot panic.
static CHUNK_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_c\d+$").expect("static chunk-suffix regex"));

/// Canonical dedup key — lowercase, alphanumeric only, whitespace trimmed.
#[must_use]
pub fn norm_label(label: &str) -> String {
    let lower = label.to_lowercase();
    let cleaned = NON_ALNUM.replace_all(&lower, "");
    cleaned.trim().to_string()
}

/// Merge nodes sharing a normalised label, rewriting edges to point at the
/// surviving ID. Self-loops created by the merge are dropped.
///
/// Selection rule for the surviving ID:
/// 1. Prefer the ID without a `_c\d+` chunk suffix.
/// 2. On a tie, prefer the shorter ID.
#[must_use]
pub fn deduplicate_by_label(nodes: &[Value], edges: &[Value]) -> (Vec<Value>, Vec<Value>) {
    use indexmap::IndexMap;

    let mut canonical: IndexMap<String, Value> = IndexMap::new();
    let mut remap: IndexMap<String, String> = IndexMap::new();

    for node in nodes {
        let label = node.get("label").and_then(Value::as_str).map_or_else(
            || {
                node.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            },
            str::to_string,
        );
        let key = norm_label(&label);
        if key.is_empty() {
            continue;
        }
        let new_id = node
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if let Some(existing) = canonical.get(&key).cloned() {
            let existing_id = existing
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let has_suffix = CHUNK_SUFFIX.is_match(&new_id);
            let existing_has_suffix = CHUNK_SUFFIX.is_match(&existing_id);
            let new_wins = (existing_has_suffix && !has_suffix) || new_id.len() < existing_id.len();
            if new_wins {
                remap.insert(existing_id, new_id);
                canonical.insert(key, node.clone());
            } else {
                remap.insert(new_id, existing_id);
            }
        } else {
            canonical.insert(key, node.clone());
        }
    }

    if remap.is_empty() {
        return (nodes.to_vec(), edges.to_vec());
    }

    let deduped_nodes: Vec<Value> = canonical.into_values().collect();
    let mut deduped_edges: Vec<Value> = Vec::with_capacity(edges.len());
    for edge in edges {
        let Some(map) = edge.as_object() else {
            deduped_edges.push(edge.clone());
            continue;
        };
        let mut new_edge = map.clone();
        if let Some(s) = new_edge
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string)
            && let Some(target) = remap.get(&s)
        {
            new_edge.insert("source".to_string(), Value::String(target.clone()));
        }
        if let Some(t) = new_edge
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_string)
            && let Some(target) = remap.get(&t)
        {
            new_edge.insert("target".to_string(), Value::String(target.clone()));
        }
        let src = new_edge.get("source").and_then(Value::as_str).unwrap_or("");
        let tgt = new_edge.get("target").and_then(Value::as_str).unwrap_or("");
        if src != tgt {
            deduped_edges.push(Value::Object(new_edge));
        }
    }

    (deduped_nodes, deduped_edges)
}
