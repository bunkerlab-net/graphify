//! Semantic-extraction cache: per-source-file nodes/edges/hyperedges.

use std::collections::HashSet;
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

/// Options for [`save_semantic_cache`].
///
/// Bundles the write-behaviour flags (Python keyword-only args) so the function
/// keeps a four-argument `(nodes, edges, hyperedges, root)` core instead of a
/// positional tail of a `bool`, an `Option<&[PathBuf]>`, and an `Option<&str>`.
#[derive(Default, Clone, Copy)]
pub struct SemanticCacheOptions<'a> {
    /// Concatenate with any existing entry instead of overwriting (#1715).
    pub merge_existing: bool,
    /// When `Some`, only these files may be used as cache-write keys (#1757).
    pub allowed_source_files: Option<&'a [PathBuf]>,
    /// Cache namespace selector: `None` → `cache/semantic/`; `Some("deep")` →
    /// `cache/semantic-deep/`, so deep-mode results never shadow standard ones
    /// (and vice versa) for the same content (#1894).
    pub mode: Option<&'a str>,
}

/// The cache namespace (kind) for a given semantic `mode`. Centralised so the
/// read path ([`check_semantic_cache`]) and write path ([`save_semantic_cache`])
/// can never diverge.
#[must_use]
pub fn semantic_kind(mode: Option<&str>) -> String {
    mode.map_or_else(|| "semantic".to_string(), |m| format!("semantic-{m}"))
}

/// Check the semantic extraction cache for a list of file paths.
///
/// For each path, loads the cached `nodes`/`edges`/`hyperedges` if its hash
/// matches; otherwise records the path in `uncached_files`. `mode` selects the
/// cache namespace (see [`SemanticCacheOptions::mode`]): `None` reads
/// `cache/semantic/`, byte-identical to the historical behaviour.
#[must_use]
pub fn check_semantic_cache(
    files: &[String],
    root: &Path,
    mode: Option<&str>,
) -> SemanticCacheSplit {
    let kind = semantic_kind(mode);
    let mut split = SemanticCacheSplit::default();
    for fpath in files {
        let mut p = PathBuf::from(fpath);
        if !p.is_absolute() {
            p = root.join(&p);
        }
        if let Some(Value::Object(map)) = load_cached(&p, root, &kind, None) {
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

/// Bucket nodes/edges/hyperedges by their non-empty `source_file` field.
type SemanticBuckets = (Vec<Value>, Vec<Value>, Vec<Value>);

fn bucket_by_source_file(
    nodes: &[Value],
    edges: &[Value],
    hyperedges: &[Value],
) -> IndexMap<String, SemanticBuckets> {
    let mut by_file: IndexMap<String, SemanticBuckets> = IndexMap::new();
    let mut push = |items: &[Value], slot: usize| {
        for it in items {
            if let Some(src) = it.get("source_file").and_then(Value::as_str)
                && !src.is_empty()
            {
                let bucket = by_file.entry(src.to_string()).or_default();
                match slot {
                    0 => bucket.0.push(it.clone()),
                    1 => bucket.1.push(it.clone()),
                    _ => bucket.2.push(it.clone()),
                }
            }
        }
    };
    push(nodes, 0);
    push(edges, 1);
    push(hyperedges, 2);
    by_file
}

/// True when a `source_file` group is skipped by the write loop: its path is not
/// a real file (ghost path) or is out-of-scope per the #1757 allowlist.
fn group_skipped(fpath: &str, allowed: &HashSet<PathBuf>, root_path: &Path) -> bool {
    let p = resolved_source_path(Path::new(fpath), root_path);
    !p.is_file() || !allowed.contains(&p)
}

/// Hashable-scalar key for a JSON id/endpoint value, mirroring Python's use of
/// the raw value in a `set`. Type-prefixed so the string `"1"` and the number
/// `1` stay distinct (as they are in Python). `None` for arrays/objects, which
/// are unhashable in Python — callers treat those as "not a match" so an
/// untrusted result cannot make the prune misbehave.
fn scalar_key(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(format!("s:{s}")),
        Value::Number(n) => Some(format!("n:{n}")),
        Value::Bool(b) => Some(format!("b:{b}")),
        Value::Null => Some("null".to_string()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// Node ids that live in VALID retained entries of the `kind` namespace and will
/// still be present after this save (#1916). An on-disk entry is loaded on replay
/// only when its bucket `source_file` resolves to a live file whose current
/// content hash still matches the entry filename — the `load_cached` liveness
/// contract — so a stale/orphaned entry is ignored. Entries for files IN the
/// current batch survive only under `merge_existing` (otherwise they are
/// overwritten, and their fresh ids are already in `written_ids`); untouched
/// entries always survive. A reference to one of these ids is therefore NOT
/// dangling even when the current batch mis-attributes the id to a skipped
/// group, so they are unioned into `written_ids` before pruning.
///
/// DIVERGENCE (#1916): graphify-py `cache.py:665-681` builds its id sets from the
/// current batch alone and never consults retained entries, so it prunes an edge
/// whose endpoint survives in another cache entry — dropping a valid relationship
/// on replay. We consult them (AGENTS.md: fix reference bugs, do not replicate).
fn retained_node_ids(
    root: &Path,
    root_path: &Path,
    kind: &str,
    batch_paths: &HashSet<PathBuf>,
    merge_existing: bool,
) -> HashSet<String> {
    let mut ids = HashSet::new();
    let Ok(dir) = crate::paths::cache_dir(root, kind) else {
        return ids;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return ids;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(nodes) = value.get("nodes").and_then(Value::as_array) else {
            continue;
        };
        let Some(sf) = nodes
            .iter()
            .find_map(|n| n.get("source_file").and_then(Value::as_str))
        else {
            continue;
        };
        let src = resolved_source_path(Path::new(sf), root_path);
        // Liveness: only entries that replay would actually load count.
        if !src.is_file() || crate::hash::file_hash(&src, root, None).ok().as_deref() != Some(stem)
        {
            continue;
        }
        // An in-batch file is overwritten (fresh ids already tracked) unless the
        // caller unions with the prior entry via `merge_existing`.
        if batch_paths.contains(&src) && !merge_existing {
            continue;
        }
        for n in nodes {
            if let Some(id) = n.get("id").and_then(scalar_key) {
                ids.insert(id);
            }
        }
    }
    ids
}

/// Dangling-reference pruning (#1916). A node group skipped by the write loop
/// (ghost path or out-of-scope) contributes node ids that never reach the cache;
/// an edge/hyperedge in an ALLOWED group referencing such an id would be written
/// verbatim and dangle forever on replay. Drop those references. Gated on the
/// allowlist so unscoped callers stay byte-identical.
fn prune_dangling_refs(
    by_file: &mut IndexMap<String, SemanticBuckets>,
    allowed: &HashSet<PathBuf>,
    root_path: &Path,
    retained: &HashSet<String>,
) {
    let mut skipped_ids: HashSet<String> = HashSet::new();
    let mut written_ids: HashSet<String> = HashSet::new();
    for (fpath, (n, _, _)) in &*by_file {
        let target = if group_skipped(fpath, allowed, root_path) {
            &mut skipped_ids
        } else {
            &mut written_ids
        };
        for node in n {
            if let Some(id) = node.get("id").and_then(scalar_key) {
                target.insert(id);
            }
        }
    }
    // An id that still reaches the cache is not dangling: either it is defined in
    // a written group of THIS batch (duplicate attribution across a skipped and a
    // written group), or it lives in a valid retained entry that survives the save
    // (`retained_node_ids`). Don't over-prune references to those.
    for id in written_ids.iter().chain(retained) {
        skipped_ids.remove(id);
    }
    if skipped_ids.is_empty() {
        return;
    }
    // An endpoint that is not a hashable scalar is left alone (Python's `x in
    // set` raises TypeError, caught as "not dangling").
    let endpoint_dangles = |e: &Value, key: &str| {
        e.get(key)
            .and_then(scalar_key)
            .is_some_and(|k| skipped_ids.contains(&k))
    };
    // A hyperedge with ANY non-scalar member is kept whole: Python's
    // `set(members)` raises on an unhashable member, so `hyperedge_dangles`
    // returns False for the whole hyperedge rather than pruning on the rest.
    let hyperedge_dangles = |h: &Value| -> bool {
        let Some(members) = h.get("nodes").and_then(Value::as_array) else {
            return false;
        };
        let mut keys = Vec::with_capacity(members.len());
        for m in members {
            match scalar_key(m) {
                Some(k) => keys.push(k),
                None => return false, // unhashable member → keep whole hyperedge
            }
        }
        keys.iter().any(|k| skipped_ids.contains(k))
    };
    for (fpath, (_, e, h)) in by_file.iter_mut() {
        if group_skipped(fpath, allowed, root_path) {
            continue;
        }
        e.retain(|edge| !(endpoint_dangles(edge, "source") || endpoint_dangles(edge, "target")));
        h.retain(|hyper| !hyperedge_dangles(hyper));
    }
}

/// Save semantic extraction results to the cache, keyed by `source_file`.
///
/// Buckets the input nodes/edges/hyperedges by their `source_file` field and
/// writes one cache entry per source file. Returns the number of source files
/// actually cached. See [`SemanticCacheOptions`] for the write-behaviour flags
/// (`merge_existing` #1715, `allowed_source_files` #1757, `mode` #1894).
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
    opts: SemanticCacheOptions<'_>,
) -> Result<usize, CacheError> {
    let SemanticCacheOptions {
        merge_existing,
        allowed_source_files,
        mode,
    } = opts;
    let kind = semantic_kind(mode);
    let mut by_file = bucket_by_source_file(nodes, edges, hyperedges);

    let root_path = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let allowed_paths: Option<HashSet<PathBuf>> = allowed_source_files.map(|allowed| {
        allowed
            .iter()
            .map(|p| resolved_source_path(p, &root_path))
            .collect()
    });
    if let Some(allowed) = &allowed_paths {
        // Files the write loop will actually overwrite (real + in-scope). A
        // skipped ghost/out-of-scope group leaves its prior entry untouched, so
        // it is NOT a batch-overwrite and its ids still count as retained.
        let batch_paths: HashSet<PathBuf> = by_file
            .keys()
            .map(|fpath| resolved_source_path(Path::new(fpath), &root_path))
            .filter(|p| p.is_file() && allowed.contains(p))
            .collect();
        let retained = retained_node_ids(root, &root_path, &kind, &batch_paths, merge_existing);
        prune_dangling_refs(&mut by_file, allowed, &root_path, &retained);
    }

    let mut saved = 0;
    for (fpath, (n, e, h)) in &by_file {
        let p = resolved_source_path(Path::new(fpath), &root_path);
        if !p.is_file() {
            continue;
        }
        // A model may mint semantic nodes that mention another corpus file, but
        // it must not replace that file's cache entry unless the file was part
        // of the current extraction batch (#1757).
        if let Some(allowed) = &allowed_paths
            && !allowed.contains(&p)
        {
            eprintln!(
                "[graphify] warning: semantic cache skipped out-of-scope \
                 source_file '{fpath}'; the file was not dispatched for extraction"
            );
            continue;
        }
        let payload = if merge_existing {
            // Accumulate a prior slice (a large file split across chunks)
            // instead of overwriting it: prev + new, in order (#1715).
            let prev = load_cached(&p, root, &kind, None);
            let prev_obj = prev.as_ref().and_then(Value::as_object);
            let merged = |key: &str, new: &[Value]| -> Vec<Value> {
                let mut out: Vec<Value> = prev_obj
                    .and_then(|m| m.get(key))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                out.extend(new.iter().cloned());
                out
            };
            serde_json::json!({
                "nodes": merged("nodes", n),
                "edges": merged("edges", e),
                "hyperedges": merged("hyperedges", h),
            })
        } else {
            serde_json::json!({
                "nodes": n,
                "edges": e,
                "hyperedges": h,
            })
        };
        save_cached(&p, &payload, root, &kind, None)?;
        saved += 1;
    }
    Ok(saved)
}

/// Resolve a `source_file` value to an absolute, canonical path for scope
/// comparison: absolute values pass through, relative ones anchor at
/// `root_path`; canonicalisation falls back to a lexical absolute path for
/// inaccessible paths or a symlink loop from an untrusted result (#1757).
fn resolved_source_path(value: &Path, root_path: &Path) -> PathBuf {
    let path = if value.is_absolute() {
        value.to_path_buf()
    } else {
        root_path.join(value)
    };
    path.canonicalize()
        .or_else(|_| std::path::absolute(&path))
        .unwrap_or(path)
}

/// Remove orphaned semantic cache entries, returning the count pruned.
///
/// The semantic cache is content-hash-keyed (`{file_hash}.json` under
/// `cache/semantic/`) and deliberately UNVERSIONED — entries are produced by the
/// LLM from file contents, so invalidating them on every release would re-bill
/// extraction. Because it is unversioned it is never swept by the AST
/// version-cleanup, so every content change or file deletion leaves a permanent
/// orphan that accumulates unbounded (#1527).
///
/// This sweeps every `cache/semantic*/` namespace — `semantic/`, the `--mode
/// deep` `semantic-deep/`, and any future `semantic-<mode>/` (#1894) — and
/// deletes any entry whose stem (the content
/// hash) is not in `live_hashes` — the hashes of the current live document set.
/// Both namespaces are pruned against the SAME live set: liveness is
/// content-based and mode-independent, so a hash live for one namespace is live
/// for both; skipping the deep namespace would re-grow the unbounded-orphan
/// problem (#1527). `*.tmp` atomic-write temporaries are skipped, and only these
/// directories are touched (never `cache/ast/**`). Best-effort: each unlink
/// failure is ignored. Mirrors Python `prune_semantic_cache`.
#[must_use]
pub fn prune_semantic_cache<S: std::hash::BuildHasher>(
    root: &Path,
    live_hashes: &HashSet<String, S>,
) -> usize {
    let mut pruned = 0;
    // Every semantic namespace, enumerated from disk so a new `--mode` is pruned
    // without a hard-coded name (#1894).
    for semantic_dir in crate::paths::semantic_cache_dirs(root) {
        let Ok(entries) = std::fs::read_dir(&semantic_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Only `*.json`; `*.tmp` atomic-write temporaries are left untouched.
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if live_hashes.contains(stem) {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                pruned += 1;
            }
        }
    }
    pruned
}
