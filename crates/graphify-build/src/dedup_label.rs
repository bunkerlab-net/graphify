//! Label-canonical node deduplication. Ports `build._norm_label` +
//! `deduplicate_by_label`.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

// `\W` in the Rust `regex` crate is Unicode-aware by default, so `[\W_ ]+`
// collapses runs of non-word characters while preserving CJK and other
// Unicode letters — matching Python's
// `re.sub(r"[\W_ ]+", " ", s, flags=re.UNICODE)`.
#[allow(clippy::expect_used)] // literal pattern; build cannot panic.
static NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\W_ ]+").expect("static non-word regex"));

#[allow(clippy::expect_used)] // literal pattern; build cannot panic.
static CHUNK_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_c\d+$").expect("static chunk-suffix regex"));

/// Canonical dedup key — Unicode-aware, preserves CJK/word characters.
///
/// Mirrors Python's
/// `re.sub(r"[\W_ ]+", " ", unicodedata.normalize("NFKC", label).casefold(),
/// flags=re.UNICODE).strip()`. Uses the `caseless` crate's full Unicode
/// case folding (Python's `str.casefold()`) rather than
/// `str::to_lowercase` so identifiers like `ß` and the Greek final
/// sigma fold identically across the two languages.
#[must_use]
pub fn norm_label(label: &str) -> String {
    let nfkc: String = label.nfkc().collect();
    let folded: String = caseless::default_case_fold_str(&nfkc);
    let cleaned = NON_WORD.replace_all(&folded, " ");
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
            // Match Python's branching: suffix presence dominates length.
            let new_wins = if has_suffix == existing_has_suffix {
                new_id.len() < existing_id.len()
            } else {
                !has_suffix
            };
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
