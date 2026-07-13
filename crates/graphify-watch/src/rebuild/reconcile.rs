//! Source-path reconciliation for incremental / full rebuilds.
//!
//! Ports `_StoredSourcePaths`, `_reconcile_existing_graph`, and
//! `_rebase_relative_source_files` from `graphify-py/graphify/watch.py`
//! (commit 8d8d2b8, "reconcile removed and renamed sources").
//!
//! The reconciliation merges a fresh AST extraction with the entries preserved
//! from the prior `graph.json`, evicting nodes/edges/hyperedges whose source
//! file was re-extracted, deleted, or renamed away. Source identity is resolved
//! through [`StoredSourcePaths`], which understands a legacy `.graphify_root`
//! marker so a graph built under a different invocation root still reconciles.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_build::norm_source_file;
use serde_json::{Value, json};

/// Lexically normalise a POSIX-style path string, mirroring Python
/// `posixpath.normpath`: collapse `.`, `..`, and redundant `/`. No filesystem
/// access, no symlink resolution. An empty string becomes `.`.
#[must_use]
pub fn posix_normpath(input: &str) -> String {
    if input.is_empty() {
        return ".".to_string();
    }
    let is_abs = input.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in input.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                let last_is_parent = parts.last().is_some_and(|p| *p == "..");
                if !is_abs && (parts.is_empty() || last_is_parent) {
                    parts.push("..");
                } else if !parts.is_empty() && !last_is_parent {
                    parts.pop();
                }
                // absolute path with nothing to pop -> ".." is dropped (stays at root).
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if is_abs {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Absolute, lexically normalised POSIX form of `path`, mirroring Python
/// `Path(os.path.abspath(path)).as_posix()`.
///
/// A relative path is joined onto the current working directory. Unlike
/// [`Path::canonicalize`], this never touches the filesystem and never resolves
/// symlinks, so a deleted or renamed source still yields a stable identity
/// (the exact case the reconciliation must handle).
#[must_use]
pub fn lexical_abs(path: &Path) -> String {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    };
    posix_normpath(&joined.to_string_lossy().replace('\\', "/"))
}

/// `.resolve()`-equivalent for an existing directory: canonicalise (resolving
/// symlinks like Python's `Path.resolve()`), falling back to a lexical absolute
/// path when the target does not exist.
fn resolve_or_abs(path: &Path) -> String {
    path.canonicalize().map_or_else(
        |_| lexical_abs(path),
        |p| p.to_string_lossy().replace('\\', "/"),
    )
}

/// POSIX form of a filesystem root, lexically normalised.
fn root_posix(root: &Path) -> String {
    posix_normpath(&root.to_string_lossy().replace('\\', "/"))
}

/// Whether the POSIX identity `id` is `root` or lies beneath it.
fn is_relative_to(id: &str, root: &Path) -> bool {
    let root_norm = root_posix(root);
    id == root_norm || id.starts_with(&format!("{root_norm}/"))
}

/// `normalize` helper: `_nsf(source_file)` for `None`/empty inputs returns
/// `None`; otherwise the backslash-normalised (optionally root-relative) form.
fn nsf_opt(source_file: Option<&str>, root: Option<&str>) -> Option<String> {
    let sf = source_file?;
    if sf.is_empty() {
        return None;
    }
    let normalized = norm_source_file(sf, root);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Absolute POSIX identity of `source_file`, resolving a relative value against
/// `root`. Mirrors `_StoredSourcePaths.absolute_identity` (no `self` state).
fn absolute_identity(source_file: Option<&str>, root: &Path) -> Option<String> {
    let normalized = nsf_opt(source_file, None)?;
    let normed = posix_normpath(&normalized);
    let source_path = Path::new(&normed);
    let abs = if source_path.is_absolute() {
        source_path.to_path_buf()
    } else {
        root.join(source_path)
    };
    Some(lexical_abs(&abs))
}

/// Resolve `source_file` values across the current and legacy graph roots.
///
/// Ports `_StoredSourcePaths`.
pub struct StoredSourcePaths {
    project_root: PathBuf,
    watch_root: PathBuf,
    existing_source_root: PathBuf,
    /// True when the prior graph's relative `source_file` values are relative to
    /// the watched root rather than the project root (legacy `.graphify_root`).
    legacy_watch_relative: bool,
}

impl StoredSourcePaths {
    /// Build the resolver from the prior graph + the `.graphify_root` marker.
    #[must_use]
    pub fn new(existing: &Value, out: &Path, project_root: &Path, watch_root: &Path) -> Self {
        let mut existing_source_root = project_root.to_path_buf();
        let mut relative_marker_prefix: Option<String> = None;

        let root_marker = out.join(".graphify_root");
        if let Ok(raw) = std::fs::read_to_string(&root_marker) {
            let saved = raw.trim();
            if !saved.is_empty() {
                let saved_root = Path::new(saved);
                if saved_root.is_absolute() {
                    existing_source_root = PathBuf::from(resolve_or_abs(saved_root));
                } else if let Ok(cwd) = std::env::current_dir() {
                    let invocation_root = cwd.canonicalize().unwrap_or(cwd);
                    let joined_resolved = resolve_or_abs(&invocation_root.join(saved_root));
                    if joined_resolved == root_posix(watch_root) {
                        existing_source_root.clone_from(&invocation_root);
                        relative_marker_prefix = Some(posix_normpath(&saved.replace('\\', "/")));
                    }
                }
            }
        }

        let legacy_watch_relative = match relative_marker_prefix.as_deref() {
            None | Some(".") => false,
            Some(prefix) => {
                let mut has_project_relative_source = false;
                'outer: for bucket in ["nodes", "links", "edges", "hyperedges"] {
                    let Some(items) = existing.get(bucket).and_then(Value::as_array) else {
                        continue;
                    };
                    for item in items {
                        let stored = nsf_opt(item.get("source_file").and_then(Value::as_str), None);
                        let Some(stored) = stored else { continue };
                        if Path::new(&stored).is_absolute() {
                            continue;
                        }
                        let normalized = posix_normpath(&stored);
                        if normalized == prefix || normalized.starts_with(&format!("{prefix}/")) {
                            has_project_relative_source = true;
                            break 'outer;
                        }
                    }
                }
                !has_project_relative_source
            }
        };

        Self {
            project_root: project_root.to_path_buf(),
            watch_root: watch_root.to_path_buf(),
            existing_source_root,
            legacy_watch_relative,
        }
    }

    /// Project-relative, lexically normalised form of a stored `source_file`.
    #[must_use]
    pub fn normalize(&self, source_file: Option<&str>) -> Option<String> {
        nsf_opt(source_file, Some(&self.project_root.to_string_lossy())).map(|n| posix_normpath(&n))
    }

    /// Absolute POSIX identity, honouring the legacy-watch-relative fallback.
    #[must_use]
    pub fn identity(&self, source_file: Option<&str>) -> Option<String> {
        let normalized = nsf_opt(source_file, None);
        if let Some(ref n) = normalized
            && !Path::new(n).is_absolute()
            && self.legacy_watch_relative
        {
            return absolute_identity(source_file, &self.watch_root);
        }
        absolute_identity(source_file, &self.existing_source_root)
    }

    /// Whether `source_file`'s identity lies within the watched root.
    #[must_use]
    pub fn in_watch_root(&self, source_file: Option<&str>) -> bool {
        self.identity(source_file)
            .is_some_and(|id| is_relative_to(&id, &self.watch_root))
    }

    /// Absolute identity of `path` under `project_root` (for `current_sources`).
    #[must_use]
    pub fn absolute_identity_for(&self, path: &Path) -> Option<String> {
        absolute_identity(Some(&path.to_string_lossy()), &self.project_root)
    }

    /// Whether an item's source identity is in the eviction set.
    fn is_evicted(&self, item: &Value, identities: &HashSet<String>) -> bool {
        self.identity(item.get("source_file").and_then(Value::as_str))
            .is_some_and(|id| identities.contains(&id))
    }

    /// Rewrite a preserved item's `source_file` to the project-relative form it
    /// would have if freshly extracted (or leave the absolute identity when it
    /// lies outside the project root).
    fn rebase_preserved(&self, item: &mut Value) {
        let Some(identity) = self.identity(item.get("source_file").and_then(Value::as_str)) else {
            return;
        };
        let Some(map) = item.as_object_mut() else {
            return;
        };
        if !is_relative_to(&identity, &self.watch_root) {
            if let Some(normalized) = self.normalize(map.get("source_file").and_then(Value::as_str))
            {
                map.insert("source_file".to_string(), Value::String(normalized));
            }
            return;
        }
        let rebased = relative_to(&identity, &self.project_root).unwrap_or(identity);
        map.insert("source_file".to_string(), Value::String(rebased));
    }
}

/// `Path(id).relative_to(root).as_posix()`, or `None` when `id` is outside `root`.
fn relative_to(id: &str, root: &Path) -> Option<String> {
    let root_norm = root_posix(root);
    if id == root_norm {
        return Some(".".to_string());
    }
    id.strip_prefix(&format!("{root_norm}/"))
        .map(str::to_string)
}

/// Rebase cache-root-relative `source_file` values onto the project root.
///
/// Ports `_rebase_relative_source_files`.
pub fn rebase_relative_source_files(payload: &mut Value, source_root: &Path, target_root: &Path) {
    if source_root == target_root {
        return;
    }
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    // Buckets `nodes`/`edges`/`hyperedges` mirror graphify-py's
    // `_rebase_relative_source_files` exactly. This runs on the FRESH extraction
    // (`result`), whose edge bucket is always `edges` — the `links` key only
    // appears after the no-cluster writer renames it, long after this call — so a
    // `links` bucket here would never match and adding one diverges from Python.
    for bucket in ["nodes", "edges", "hyperedges"] {
        let Some(items) = obj.get_mut(bucket).and_then(Value::as_array_mut) else {
            continue;
        };
        for item in items {
            let Some(map) = item.as_object_mut() else {
                continue;
            };
            let Some(source) = map.get("source_file").and_then(Value::as_str) else {
                continue;
            };
            if source.is_empty() || Path::new(source).is_absolute() {
                continue;
            }
            let abs = source_root.join(source);
            let abs_posix = lexical_abs(&abs);
            if let Some(rel) = relative_to(&abs_posix, target_root) {
                map.insert("source_file".to_string(), Value::String(rel));
            }
        }
    }
}

/// Outcome of [`reconcile_existing_graph`].
pub(crate) struct ReconcileOutcome {
    /// The prior graph JSON (as loaded), or `Value::Null` when absent/corrupt.
    pub existing_graph_data: Value,
}

/// Merge fresh extraction with preserved graph entries and evict stale sources.
///
/// Ports `_reconcile_existing_graph`. Mutates `result` in place with the merged
/// graph and `deleted_paths` with any newly-detected removed/renamed sources.
#[allow(clippy::too_many_arguments)] // mirrors the Python signature's reconciliation inputs
pub(crate) fn reconcile_existing_graph(
    existing_graph_path: &Path,
    result: &mut Value,
    out: &Path,
    project_root: &Path,
    watch_root: &Path,
    code_files: &[PathBuf],
    extract_targets: &[PathBuf],
    full_rebuild: bool,
    deleted_paths: &mut Vec<String>,
    deleted_source_identities: &HashSet<String>,
) -> ReconcileOutcome {
    let mut outcome = ReconcileOutcome {
        existing_graph_data: Value::Null,
    };
    if !existing_graph_path.exists() {
        return outcome;
    }
    if graphify_security::check_graph_file_size_cap(existing_graph_path).is_err() {
        return outcome;
    }
    let Ok(text) = std::fs::read_to_string(existing_graph_path) else {
        return outcome;
    };
    let Ok(existing) = serde_json::from_str::<Value>(&text) else {
        return outcome;
    };
    outcome.existing_graph_data = existing.clone();

    let source_paths = StoredSourcePaths::new(&existing, out, project_root, watch_root);

    let new_ast_ids: HashSet<String> = result
        .get("nodes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let EvictionSets {
        node: node_evicted,
        edge: edge_evicted,
        hyperedge: hyperedge_evicted,
    } = compute_eviction_sets(
        &existing,
        &source_paths,
        code_files,
        extract_targets,
        full_rebuild,
        deleted_source_identities,
        deleted_paths,
    );

    let empty: Vec<Value> = Vec::new();
    let existing_nodes = existing
        .get("nodes")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let preserved = filter_preserved_nodes(
        existing_nodes,
        &source_paths,
        &new_ast_ids,
        &node_evicted,
        full_rebuild,
        code_files.is_empty(),
    );

    let mut all_ids = new_ast_ids.clone();
    all_ids.extend(
        preserved
            .iter()
            .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string)),
    );

    let preserved_edges = filter_preserved_edges(&existing, &source_paths, &all_ids, &edge_evicted);

    let new_hyper_ids: HashSet<String> = result
        .get("hyperedges")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|h| h.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let preserved_hyperedges = filter_preserved_hyperedges(
        &existing,
        &source_paths,
        &new_hyper_ids,
        &all_ids,
        &hyperedge_evicted,
    );
    merge_preserved_into_result(
        result,
        &source_paths,
        preserved,
        preserved_edges,
        preserved_hyperedges,
    );
    outcome
}

/// Nodes preserved from the prior graph: not re-emitted by the fresh AST, not
/// owned by a rebuilt source, not evicted. Mirrors the `preserved_nodes`
/// comprehension in `_reconcile_existing_graph`.
fn filter_preserved_nodes(
    existing_nodes: &[Value],
    source_paths: &StoredSourcePaths,
    new_ast_ids: &HashSet<String>,
    node_evicted: &HashSet<String>,
    full_rebuild: bool,
    code_files_empty: bool,
) -> Vec<Value> {
    existing_nodes
        .iter()
        .filter(|node| {
            let id = node.get("id").and_then(Value::as_str).unwrap_or("");
            if new_ast_ids.contains(id) {
                return false;
            }
            let is_ast = node.get("_origin").and_then(Value::as_str) == Some("ast");
            let source_file = node.get("source_file").and_then(Value::as_str);
            let owned_by_rebuild = is_ast
                && ((source_file.is_none_or(str::is_empty) && (full_rebuild || code_files_empty))
                    || (full_rebuild && source_paths.in_watch_root(source_file)));
            if owned_by_rebuild {
                return false;
            }
            !source_paths.is_evicted(node, node_evicted)
        })
        .cloned()
        .collect()
}

/// Edges preserved from the prior graph: both endpoints survive and the edge's
/// owning source was not re-extracted/deleted.
fn filter_preserved_edges(
    existing: &Value,
    source_paths: &StoredSourcePaths,
    all_ids: &HashSet<String>,
    edge_evicted: &HashSet<String>,
) -> Vec<Value> {
    existing
        .get("links")
        .or_else(|| existing.get("edges"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|edge| {
                    let src = edge.get("source").and_then(Value::as_str).unwrap_or("");
                    let tgt = edge.get("target").and_then(Value::as_str).unwrap_or("");
                    all_ids.contains(src)
                        && all_ids.contains(tgt)
                        && !source_paths.is_evicted(edge, edge_evicted)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Hyperedges preserved from the prior graph: not re-emitted, not evicted (by
/// the deletion-only hyperedge set), all members still present.
fn filter_preserved_hyperedges(
    existing: &Value,
    source_paths: &StoredSourcePaths,
    new_hyper_ids: &HashSet<String>,
    all_ids: &HashSet<String>,
    hyperedge_evicted: &HashSet<String>,
) -> Vec<Value> {
    existing
        .get("hyperedges")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|edge| {
                    if let Some(id) = edge.get("id").and_then(Value::as_str)
                        && new_hyper_ids.contains(id)
                    {
                        return false;
                    }
                    if source_paths.is_evicted(edge, hyperedge_evicted) {
                        return false;
                    }
                    let members = edge
                        .get("nodes")
                        .or_else(|| edge.get("members"))
                        .or_else(|| edge.get("node_ids"))
                        .and_then(Value::as_array);
                    if let Some(members) = members
                        && members
                            .iter()
                            .any(|m| m.as_str().is_none_or(|s| !all_ids.contains(s)))
                    {
                        return false;
                    }
                    true
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The three eviction identity sets for the reconciliation.
struct EvictionSets {
    node: HashSet<String>,
    edge: HashSet<String>,
    hyperedge: HashSet<String>,
}

/// Build the node/edge/hyperedge eviction sets and discover stale (removed or
/// renamed) sources, appending their normalised paths to `deleted_paths`.
fn compute_eviction_sets(
    existing: &Value,
    source_paths: &StoredSourcePaths,
    code_files: &[PathBuf],
    extract_targets: &[PathBuf],
    full_rebuild: bool,
    deleted_source_identities: &HashSet<String>,
    deleted_paths: &mut Vec<String>,
) -> EvictionSets {
    let current_sources: HashSet<String> = code_files
        .iter()
        .filter_map(|p| source_paths.absolute_identity_for(p))
        .collect();
    let rebuilt: HashSet<String> = extract_targets
        .iter()
        .filter_map(|p| source_paths.absolute_identity_for(p))
        .collect();

    let mut node: HashSet<String> = deleted_source_identities.clone();
    // Hyperedges are evicted only by explicit/de-facto deletions (NOT every
    // rebuilt source), else a full update would drop every preserved hyperedge
    // whose source was re-extracted (#1755). The stale-source loop below adds to
    // all three sets.
    let mut hyperedge: HashSet<String> = deleted_source_identities.clone();
    if !full_rebuild {
        node.extend(rebuilt.iter().cloned());
    }
    let mut edge: HashSet<String> = node.clone();
    edge.extend(rebuilt.iter().cloned());

    // Reconcile every rebuild against the current watched corpus. A hook change
    // list can carry only a rename destination, so explicit paths alone cannot
    // identify the stale source. Scope the comparison to the watched root so a
    // subfolder update preserves records outside that subtree.
    if let Some(nodes) = existing.get("nodes").and_then(Value::as_array) {
        for node_val in nodes {
            let source_file = node_val.get("source_file").and_then(Value::as_str);
            let Some(sf) = source_file else { continue };
            if sf.is_empty() || !graphify_extract::has_extractor(Path::new(sf)) {
                continue;
            }
            if !source_paths.in_watch_root(source_file) {
                continue;
            }
            let Some(identity) = source_paths.identity(source_file) else {
                continue;
            };
            if !current_sources.contains(&identity) {
                if let Some(normalized) = source_paths.normalize(source_file)
                    && !deleted_paths.contains(&normalized)
                {
                    deleted_paths.push(normalized);
                }
                node.insert(identity.clone());
                edge.insert(identity.clone());
                hyperedge.insert(identity);
            }
        }
    }

    EvictionSets {
        node,
        edge,
        hyperedge,
    }
}

/// Rebase preserved items to project-relative paths, then append them to the
/// fresh extraction in `result`.
fn merge_preserved_into_result(
    result: &mut Value,
    source_paths: &StoredSourcePaths,
    mut preserved_nodes: Vec<Value>,
    mut preserved_edges: Vec<Value>,
    mut preserved_hyperedges: Vec<Value>,
) {
    for item in preserved_nodes
        .iter_mut()
        .chain(preserved_edges.iter_mut())
        .chain(preserved_hyperedges.iter_mut())
    {
        source_paths.rebase_preserved(item);
    }
    let take = |result: &Value, bucket: &str| -> Vec<Value> {
        result
            .get(bucket)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut merged_nodes = take(result, "nodes");
    let mut merged_edges = take(result, "edges");
    let mut merged_hyper = take(result, "hyperedges");
    merged_nodes.extend(preserved_nodes);
    merged_edges.extend(preserved_edges);
    merged_hyper.extend(preserved_hyperedges);
    *result = json!({
        "nodes": merged_nodes,
        "edges": merged_edges,
        "hyperedges": merged_hyper,
        "input_tokens": 0,
        "output_tokens": 0,
    });
}
