//! Cross-corpus graph merging.
//!
//! Ports `graphify-py/graphify/global_graph.py`. Combines multiple per-repo
//! graphs into a single global graph stored under `~/.graphify/`.
//!
//! # Design
//!
//! The on-disk format is the `NetworkX` `node_link_data` JSON shape:
//! `{"directed": …, "multigraph": …, "graph": {}, "nodes": […], "links": […]}`.
//! Reading normalises `"links"` → `"edges"` so [`graphify_build::build_from_json`]
//! can parse it. Writing always emits `"links"` for round-trip compatibility.
//!
//! [`graphify_build::prefix_graph_for_global`] and
//! [`graphify_build::prune_repo_from_graph`] are re-exported so callers don't
//! need a direct dependency on `graphify-build`.

use std::path::{Path, PathBuf};

use graphify_build::{Graph, GraphKind};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

// Re-export the build helpers that operate on the global graph so callers only
// need one dependency.
pub use graphify_build::prefix_graph_for_global;
pub use graphify_build::prune_repo_from_graph;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by `graphify-global` operations.
#[derive(Debug, Error)]
pub enum GlobalError {
    /// The source graph file does not exist.
    #[error("graph not found: {0}")]
    GraphNotFound(PathBuf),

    /// `global_remove` was called for a repo tag not present in the manifest.
    #[error("repo '{0}' not in global graph")]
    UnknownRepo(String),

    /// An error from the build layer (graph construction).
    #[error("build error: {0}")]
    Build(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Default paths  (mirrors Python's `_GLOBAL_DIR / …`)
// ---------------------------------------------------------------------------

fn default_global_dir() -> PathBuf {
    // `home_dir` is deprecated in 1.85+ but `std::env::home_dir` is the only
    // stable way to get HOME without a third-party crate.
    #[allow(deprecated)] // reason: std::env::home_dir is the only stable approach
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".graphify")
}

/// Returns the default path of the global graph JSON file.
#[must_use]
pub fn global_graph_path() -> PathBuf {
    default_global_dir().join("global-graph.json")
}

/// Returns the default path of the global manifest JSON file.
#[must_use]
pub fn global_manifest_path() -> PathBuf {
    default_global_dir().join("global-manifest.json")
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// Per-repo entry recorded in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub added_at: String,
    pub source_path: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub source_hash: String,
}

/// Top-level manifest structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub repos: IndexMap<String, RepoEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            repos: IndexMap::new(),
        }
    }
}

fn load_manifest(path: &Path) -> Manifest {
    if path.exists()
        && let Ok(text) = std::fs::read_to_string(path)
        && let Ok(m) = serde_json::from_str(&text)
    {
        return m;
    }
    Manifest::default()
}

fn save_manifest(path: &Path, manifest: &Manifest) -> Result<(), GlobalError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, text)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Graph serialisation (NetworkX node_link_data shape)
// ---------------------------------------------------------------------------

/// Load a [`Graph`] from a NetworkX-style `node_link_data` JSON file.
///
/// Normalises `"links"` → `"edges"` before passing to
/// [`graphify_build::build_from_json`] so both key spellings are accepted.
///
/// # Errors
///
/// Returns [`GlobalError::Io`] or [`GlobalError::Json`] if the file cannot be
/// read or parsed.
pub fn load_graph_from_file(path: &Path) -> Result<Graph, GlobalError> {
    if !path.exists() {
        return Ok(Graph::new(GraphKind::Graph));
    }
    let text = std::fs::read_to_string(path)?;
    let mut data: serde_json::Map<String, Value> = serde_json::from_str(&text)?;

    // Normalise "links" → "edges" so build_from_json can parse it.
    if !data.contains_key("edges")
        && let Some(links) = data.remove("links")
    {
        data.insert("edges".to_string(), links);
    }

    let directed = data
        .get("directed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let extraction = Value::Object(data);
    graphify_build::build_from_json(extraction, directed, None)
        .map_err(|e| GlobalError::Build(e.to_string()))
}

/// Serialise a [`Graph`] to the `NetworkX` `node_link_data` JSON format and
/// write it to `path`.
///
/// # Errors
///
/// Returns [`GlobalError::Io`] or [`GlobalError::Json`] on write failure.
pub fn save_graph_to_file(path: &Path, graph: &Graph) -> Result<(), GlobalError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let nodes: Vec<Value> = graph
        .nodes()
        .map(|(id, attrs)| {
            let mut obj = serde_json::Map::new();
            obj.insert("id".to_string(), Value::String(id.clone()));
            for (k, v) in attrs {
                obj.insert(k.clone(), v.clone());
            }
            Value::Object(obj)
        })
        .collect();

    let links: Vec<Value> = graph
        .edges()
        .map(|edge| {
            let mut obj = serde_json::Map::new();
            obj.insert("source".to_string(), Value::String(edge.source.clone()));
            obj.insert("target".to_string(), Value::String(edge.target.clone()));
            for (k, v) in &edge.attrs {
                // Skip the internal _src/_tgt bookkeeping keys.
                if k != "_src" && k != "_tgt" {
                    obj.insert(k.clone(), v.clone());
                }
            }
            Value::Object(obj)
        })
        .collect();

    let payload = serde_json::json!({
        "directed": graph.kind.is_directed(),
        "multigraph": graph.kind.is_multi(),
        "graph": {},
        "nodes": nodes,
        "links": links,
    });

    let text = serde_json::to_string_pretty(&payload)?;
    std::fs::write(path, text)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// File hashing  (mirrors Python's `_file_hash`)
// ---------------------------------------------------------------------------

/// Return the first 16 hex characters of the SHA-256 digest of `path`'s
/// contents. Mirrors Python `_file_hash(path)`.
///
/// # Errors
///
/// Returns [`GlobalError::Io`] if the file cannot be read.
pub fn file_hash(path: &Path) -> Result<String, GlobalError> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(hex::encode(&digest[..8])) // 8 bytes → 16 hex chars
}

// ---------------------------------------------------------------------------
// Timestamp helper
// ---------------------------------------------------------------------------

fn utc_now_iso8601() -> String {
    use chrono::Utc;
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Summary returned by [`global_add`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSummary {
    pub repo_tag: String,
    pub nodes_added: usize,
    pub nodes_removed: usize,
    pub skipped: bool,
}

/// Add or update a project graph in the global graph.
///
/// Mirrors Python `global_add(source_path, repo_tag)`. Paths for the global
/// graph and manifest default to `~/.graphify/`; pass `graph_path` and
/// `manifest_path` explicitly in tests.
///
/// # Errors
///
/// - [`GlobalError::GraphNotFound`] if `source_path` does not exist.
/// - [`GlobalError::Io`] / [`GlobalError::Json`] on I/O or parse failures.
pub fn global_add(
    source_path: &Path,
    repo_tag: &str,
    graph_path: &Path,
    manifest_path: &Path,
) -> Result<AddSummary, GlobalError> {
    if !source_path.exists() {
        return Err(GlobalError::GraphNotFound(source_path.to_path_buf()));
    }

    let mut manifest = load_manifest(manifest_path);
    let src_hash = file_hash(source_path)?;

    // Warn if the tag previously pointed to a different path (mirrors Python
    // `print(…, file=sys.stderr)`). In Rust we use eprintln! for stderr.
    let canonical_source = source_path
        .canonicalize()
        .unwrap_or_else(|_| source_path.to_path_buf());
    if let Some(existing) = manifest.repos.get(repo_tag) {
        let prev_path = &existing.source_path;
        if !prev_path.is_empty() && *prev_path != canonical_source.to_string_lossy().as_ref() {
            eprintln!(
                "[graphify global] warning: repo tag '{repo_tag}' previously pointed to \
                 {prev_path:?}, now updating to {}. \
                 Use --as <tag> to give it a different name.",
                canonical_source.display()
            );
        }
        if existing.source_hash == src_hash {
            return Ok(AddSummary {
                repo_tag: repo_tag.to_string(),
                nodes_added: 0,
                nodes_removed: 0,
                skipped: true,
            });
        }
    }

    // Load and prefix the source graph.
    let src_graph = load_graph_from_file(source_path)?;
    let prefixed = prefix_graph_for_global(&src_graph, repo_tag);

    // Load global graph and prune stale nodes for this repo.
    let mut global = load_graph_from_file(graph_path)?;
    let removed = prune_repo_from_graph(&mut global, repo_tag);

    // Build a set of external-library labels already in the global graph
    // (nodes with no source_file) to avoid duplication.
    let external_labels: indexmap::IndexSet<String> = global
        .nodes()
        .filter(|(_, attrs)| {
            attrs
                .get("source_file")
                .is_none_or(|v| v.as_str().is_none_or(str::is_empty))
        })
        .filter_map(|(_, attrs)| attrs.get("label").and_then(Value::as_str).map(String::from))
        .collect();

    // Identify prefixed nodes that are external duplicates to skip.
    let nodes_to_skip: indexmap::IndexSet<String> = prefixed
        .nodes()
        .filter(|(_, attrs)| {
            let no_source = attrs
                .get("source_file")
                .is_none_or(|v| v.as_str().is_none_or(str::is_empty));
            let label = attrs.get("label").and_then(Value::as_str).unwrap_or("");
            no_source && !label.is_empty() && external_labels.contains(label)
        })
        .map(|(id, _)| id.clone())
        .collect();

    // Merge prefixed nodes (excluding deduplicated externals) into global graph.
    for (id, attrs) in prefixed.nodes() {
        if !nodes_to_skip.contains(id) {
            global.add_node(id, attrs.clone());
        }
    }
    for edge in prefixed.edges() {
        if !nodes_to_skip.contains(&edge.source) && !nodes_to_skip.contains(&edge.target) {
            global.add_edge(&edge.source, &edge.target, edge.attrs.clone());
        }
    }

    let added = prefixed.node_count() - nodes_to_skip.len();
    save_graph_to_file(graph_path, &global)?;

    manifest.repos.insert(
        repo_tag.to_string(),
        RepoEntry {
            added_at: utc_now_iso8601(),
            source_path: canonical_source.to_string_lossy().into_owned(),
            node_count: added,
            edge_count: prefixed.edge_count(),
            source_hash: src_hash,
        },
    );
    save_manifest(manifest_path, &manifest)?;

    Ok(AddSummary {
        repo_tag: repo_tag.to_string(),
        nodes_added: added,
        nodes_removed: removed,
        skipped: false,
    })
}

/// Remove all nodes for `repo_tag` from the global graph. Returns the count
/// removed.
///
/// # Errors
///
/// - [`GlobalError::UnknownRepo`] if `repo_tag` is not in the manifest.
/// - [`GlobalError::Io`] / [`GlobalError::Json`] on I/O or parse failures.
pub fn global_remove(
    repo_tag: &str,
    graph_path: &Path,
    manifest_path: &Path,
) -> Result<usize, GlobalError> {
    let mut manifest = load_manifest(manifest_path);
    if !manifest.repos.contains_key(repo_tag) {
        return Err(GlobalError::UnknownRepo(repo_tag.to_string()));
    }

    let mut global = load_graph_from_file(graph_path)?;
    let removed = prune_repo_from_graph(&mut global, repo_tag);
    save_graph_to_file(graph_path, &global)?;

    manifest.repos.shift_remove(repo_tag);
    save_manifest(manifest_path, &manifest)?;

    Ok(removed)
}

/// Return the repos section of the manifest.
///
/// # Errors
///
/// Never fails (returns empty map if the manifest cannot be read).
#[must_use]
pub fn global_list(manifest_path: &Path) -> IndexMap<String, RepoEntry> {
    load_manifest(manifest_path).repos
}
