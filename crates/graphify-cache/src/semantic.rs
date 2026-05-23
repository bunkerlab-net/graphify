//! Semantic-extraction cache: per-source-file nodes/edges/hyperedges.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::Value;

use crate::error::CacheError;
use crate::store::{load_cached, save_cached};

/// Output of [`check_semantic_cache`]: split into cached graph pieces and
/// the set of files that still need extraction.
#[derive(Debug, Default)]
pub struct SemanticCacheSplit {
    /// Nodes pulled from the cache for already-extracted files.
    pub cached_nodes: Vec<Value>,
    /// Edges pulled from the cache for already-extracted files.
    pub cached_edges: Vec<Value>,
    /// Hyperedges pulled from the cache for already-extracted files.
    pub cached_hyperedges: Vec<Value>,
    /// Files that have no semantic cache entry and must be re-extracted.
    pub uncached_files: Vec<String>,
}

/// Check the semantic extraction cache for a list of file paths.
///
/// For each path, loads the cached `nodes`/`edges`/`hyperedges` if its
/// hash matches; otherwise records the path in `uncached_files`.
#[must_use]
pub fn check_semantic_cache(files: &[String], root: &Path) -> SemanticCacheSplit {
    let mut split = SemanticCacheSplit::default();
    for fpath in files {
        let mut p = PathBuf::from(fpath);
        if !p.is_absolute() {
            p = root.join(&p);
        }
        if let Some(Value::Object(map)) = load_cached(&p, root, "semantic") {
            if let Some(Value::Array(ns)) = map.get("nodes") {
                split.cached_nodes.extend(ns.iter().cloned());
            }
            if let Some(Value::Array(es)) = map.get("edges") {
                split.cached_edges.extend(es.iter().cloned());
            }
            if let Some(Value::Array(hs)) = map.get("hyperedges") {
                split.cached_hyperedges.extend(hs.iter().cloned());
            }
        } else {
            split.uncached_files.push(fpath.clone());
        }
    }
    split
}

/// Save semantic extraction results to the cache, keyed by `source_file`.
///
/// Buckets the input nodes/edges/hyperedges by their `source_file` field
/// and writes one cache entry per source file. Returns the number of
/// source files actually cached.
///
/// # Errors
///
/// Returns [`CacheError::Io`] on filesystem failure or [`CacheError::Json`]
/// on serialisation failure.
pub fn save_semantic_cache(
    nodes: &[Value],
    edges: &[Value],
    hyperedges: &[Value],
    root: &Path,
) -> Result<usize, CacheError> {
    type SemanticBuckets = (Vec<Value>, Vec<Value>, Vec<Value>);
    let mut by_file: IndexMap<String, SemanticBuckets> = IndexMap::new();
    for n in nodes {
        if let Some(src) = n.get("source_file").and_then(Value::as_str)
            && !src.is_empty()
        {
            by_file
                .entry(src.to_string())
                .or_default()
                .0
                .push(n.clone());
        }
    }
    for e in edges {
        if let Some(src) = e.get("source_file").and_then(Value::as_str)
            && !src.is_empty()
        {
            by_file
                .entry(src.to_string())
                .or_default()
                .1
                .push(e.clone());
        }
    }
    for h in hyperedges {
        if let Some(src) = h.get("source_file").and_then(Value::as_str)
            && !src.is_empty()
        {
            by_file
                .entry(src.to_string())
                .or_default()
                .2
                .push(h.clone());
        }
    }

    let mut saved = 0;
    for (fpath, (n, e, h)) in &by_file {
        let mut p = PathBuf::from(fpath);
        if !p.is_absolute() {
            p = root.join(&p);
        }
        if p.is_file() {
            let payload = serde_json::json!({
                "nodes": n,
                "edges": e,
                "hyperedges": h,
            });
            save_cached(&p, &payload, root, "semantic")?;
            saved += 1;
        }
    }
    Ok(saved)
}
