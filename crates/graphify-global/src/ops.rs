//! Top-level operations: `global_add`, `global_remove`, `global_list`.

use std::path::Path;

use chrono::Utc;
use indexmap::IndexMap;
use serde_json::Value;

use graphify_build::{prefix_graph_for_global, prune_repo_from_graph};

use crate::error::GlobalError;
use crate::io::{file_hash, load_graph_from_file, save_graph_to_file};
use crate::manifest::{RepoEntry, load_manifest, save_manifest};

/// Return the current UTC time as an RFC 3339 string.
fn utc_now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

/// Summary returned by [`global_add`] describing the change applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSummary {
    /// Repo tag the change was applied to.
    pub repo_tag: String,
    /// Number of nodes added to the global graph after deduplication
    /// against existing external libraries.
    pub nodes_added: usize,
    /// Number of stale nodes for this repo that were removed before the
    /// merge.
    pub nodes_removed: usize,
    /// `true` if the source graph hash matched the previous entry and no
    /// merge was performed.
    pub skipped: bool,
}

/// Add or update a project graph in the global graph.
///
/// Mirrors Python `global_add(source_path, repo_tag)`. Paths for the
/// global graph and manifest default to `~/.graphify/`; pass `graph_path`
/// and `manifest_path` explicitly in tests.
///
/// # Errors
///
/// - [`GlobalError::GraphNotFound`] if `source_path` does not exist.
/// - [`GlobalError::Io`] / [`GlobalError::Json`] on I/O or parse
///   failures.
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

    let src_graph = load_graph_from_file(source_path)?;
    let prefixed = prefix_graph_for_global(&src_graph, repo_tag);

    let mut global = load_graph_from_file(graph_path)?;
    let removed = prune_repo_from_graph(&mut global, repo_tag);

    // Map each external library label already present in the global graph to
    // its node ID, so a duplicate external contributed by this repo can be
    // rewired onto the existing node. Last writer wins, matching the Python
    // dict comprehension.
    let mut external_labels: IndexMap<String, String> = IndexMap::new();
    for (id, attrs) in global.nodes() {
        let no_source = attrs
            .get("source_file")
            .is_none_or(|v| v.as_str().is_none_or(str::is_empty));
        if no_source
            && let Some(label) = attrs.get("label").and_then(Value::as_str)
            && !label.is_empty()
        {
            external_labels.insert(label.to_string(), id.clone());
        }
    }

    // Map each deduplicated external in the incoming prefixed graph onto the
    // existing global node so that edges incident to it can be rewired instead
    // of dropped.
    let mut remap: IndexMap<String, String> = IndexMap::new();
    for (id, attrs) in prefixed.nodes() {
        let no_source = attrs
            .get("source_file")
            .is_none_or(|v| v.as_str().is_none_or(str::is_empty));
        if no_source
            && let Some(label) = attrs.get("label").and_then(Value::as_str)
            && let Some(existing) = external_labels.get(label)
        {
            remap.insert(id.clone(), existing.clone());
        }
    }

    // Add prefixed nodes except the deduplicated externals.
    for (id, attrs) in prefixed.nodes() {
        if !remap.contains_key(id) {
            global.add_node(id, attrs.clone());
        }
    }
    // Rewire edges through the remap table; skip self-loops it may introduce.
    let mut edges_added: usize = 0;
    for edge in prefixed.edges() {
        let u = remap
            .get(&edge.source)
            .cloned()
            .unwrap_or_else(|| edge.source.clone());
        let v = remap
            .get(&edge.target)
            .cloned()
            .unwrap_or_else(|| edge.target.clone());
        if u != v {
            global.add_edge(&u, &v, edge.attrs.clone());
            edges_added += 1;
        }
    }

    let added = prefixed.node_count() - remap.len();
    save_graph_to_file(graph_path, &global)?;

    manifest.repos.insert(
        repo_tag.to_string(),
        RepoEntry {
            added_at: utc_now_iso8601(),
            source_path: canonical_source.to_string_lossy().into_owned(),
            node_count: added,
            edge_count: edges_added,
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

/// Remove every node tagged with `repo_tag` from the global graph and
/// drop the repo from the manifest. Returns the count of nodes removed.
///
/// # Errors
///
/// - [`GlobalError::UnknownRepo`] if `repo_tag` is not in the manifest.
/// - [`GlobalError::Io`] / [`GlobalError::Json`] on I/O or parse
///   failures.
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
/// Reading is best-effort: returns an empty map if the manifest cannot
/// be read or parsed.
#[must_use]
pub fn global_list(manifest_path: &Path) -> IndexMap<String, RepoEntry> {
    load_manifest(manifest_path).repos
}
