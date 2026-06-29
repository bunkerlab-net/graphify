//! Per-file extraction cache helpers (thin wrappers around graphify-cache).
#![allow(clippy::case_sensitive_file_extension_comparisons)]

use super::{get_extractor, with_xaml_extract_root};
use crate::types::{Edge, FileResult, Node, RawCall};
use serde_json::Value;
use std::path::Path;

/// Serialise a `FileResult` to a `serde_json::Value` suitable for caching.
///
/// Converts nodes, edges, and `raw_calls` to JSON arrays. Used as the write side of the
/// graphify-cache pair; see `value_to_file_result` for the read side.
fn file_result_to_value(result: &FileResult) -> Value {
    let nodes: Vec<Value> = result
        .nodes
        .iter()
        .map(|n| serde_json::to_value(n).unwrap_or(Value::Null))
        .collect();
    let edges: Vec<Value> = result
        .edges
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    let raw_calls: Vec<Value> = result
        .raw_calls
        .iter()
        .map(|rc| {
            serde_json::json!({
                "caller_nid": rc.caller_nid,
                "callee": rc.callee,
                "is_member_call": rc.is_member_call,
                "source_file": rc.source_file,
                "source_location": rc.source_location,
                "receiver": rc.receiver,
            })
        })
        .collect();
    serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "raw_calls": raw_calls,
    })
}

/// Deserialise a cached `serde_json::Value` back into a `FileResult`.
///
/// Missing or malformed sub-fields silently fall back to empty `Vec`s.
/// Counterpart to `file_result_to_value`.
fn value_to_file_result(v: &Value) -> FileResult {
    let nodes = v
        .get("nodes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|n| serde_json::from_value::<Node>(n.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let edges = v
        .get("edges")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|e| serde_json::from_value::<Edge>(e.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let raw_calls = v
        .get("raw_calls")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|rc| {
                    Some(RawCall {
                        caller_nid: rc.get("caller_nid")?.as_str()?.to_string(),
                        callee: rc.get("callee")?.as_str()?.to_string(),
                        is_member_call: rc
                            .get("is_member_call")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        source_file: rc
                            .get("source_file")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        source_location: rc
                            .get("source_location")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        // `receiver` (#1356) reads back as `None` when absent.
                        // Safe without a Swift cache bypass or schema-version
                        // check: the AST cache is namespaced by crate version
                        // (`cache/ast/v{version}/` via graphify-cache's
                        // EXTRACTOR_VERSION), so a pre-`receiver` entry sits
                        // under an older version dir `load_cached` never reads,
                        // invalidated by the version bump that shipped the field.
                        receiver: rc
                            .get("receiver")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    FileResult {
        nodes,
        edges,
        raw_calls,
        error: None,
    }
}

// ── Extract a single file (with cache) ───────────────────────────────────────

/// File suffixes whose per-file AST extraction is never cached: their cross-file
/// import resolution depends on sibling files that can appear or change between
/// runs, so a cached result would serve a stale (unresolved) import edge.
/// Mirrors Python `_JS_CACHE_BYPASS_SUFFIXES`.
const JS_CACHE_BYPASS_SUFFIXES: [&str; 7] = ["js", "jsx", "mjs", "ts", "tsx", "vue", "svelte"];

/// Extract a single file, returning a cached result when available.
///
/// Looks up the on-disk AST cache first; on a miss, dispatches to the language-specific
/// extractor and writes the result back to the cache. Files with no matching extractor
/// return an empty `FileResult` rather than an error.
pub(super) fn extract_single_file(path: &Path, effective_root: &Path) -> FileResult {
    // JS/TS files bypass the AST cache so workspace/sibling import resolution is
    // recomputed each run (#9a7dbfb): a result cached while a sibling was absent
    // would otherwise pin a stale unresolved import edge.
    let bypass_cache = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| JS_CACHE_BYPASS_SUFFIXES.contains(&ext));

    if !bypass_cache && let Some(v) = graphify_cache::load_cached(path, effective_root, "ast") {
        return value_to_file_result(&v);
    }

    let Some(extractor) = get_extractor(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: None,
        };
    };

    let result = with_xaml_extract_root(Some(effective_root), || extractor(path));
    if !bypass_cache && result.error.is_none() {
        let v = file_result_to_value(&result);
        // best-effort save; ignore failures
        let _ = graphify_cache::save_cached(path, &v, effective_root, "ast");
    }
    result
}

// ── Cross-file Python import resolution helpers ───────────────────────────────
