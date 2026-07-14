//! `merge-driver` and `merge-graphs` commands — union-merge graph JSON files.

use anyhow::Result;

use crate::cli::graphify_out_dir;

/// Maximum file size (bytes) accepted by [`read_graph_capped`] — 50 MiB.
pub(crate) const MERGE_MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Maximum node count accepted after a merge before the operation is aborted.
pub(crate) const MERGE_MAX_NODES: usize = 100_000;

/// Read and parse `graph.json`, refusing files larger than `MERGE_MAX_BYTES`.
///
/// Guards against accidentally merging an enormous file that would exceed
/// the 100 k-node cap checked later by `cmd_merge_driver`.
pub(crate) fn read_graph_capped(path: &std::path::Path) -> Result<serde_json::Value> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("cannot stat {}: {e}", path.display()))?;
    if metadata.len() > MERGE_MAX_BYTES {
        anyhow::bail!(
            "graph.json {} is {} bytes, exceeds {}-byte cap",
            path.display(),
            metadata.len(),
            MERGE_MAX_BYTES
        );
    }
    let contents = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    Ok(value)
}

/// Union-merge two graph JSON values, deduplicating nodes by id (last writer wins).
///
/// Edges and hyperedges from both graphs are concatenated without deduplication.
/// Mirrors the Python `_merge_graphs` helper in `__main__.py`. Operating on raw
/// JSON arrays (not networkx graphs) makes this immune to the #1606 "all graphs
/// must be directed or undirected" mismatch: inputs may freely mix
/// directed/undirected/multi shapes.
pub(crate) fn merge_two_graphs(
    a: serde_json::Value,
    b: serde_json::Value,
) -> Result<serde_json::Value> {
    use serde_json::Value;
    let mut nodes_by_id: indexmap::IndexMap<String, Value> = indexmap::IndexMap::new();
    let mut edges: Vec<Value> = Vec::new();
    let mut hyperedges: Vec<Value> = Vec::new();
    for graph in [a, b] {
        let obj = graph
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("graph is not a JSON object"))?
            .clone();
        let nodes = obj
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for node in nodes {
            let id = node
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !id.is_empty() {
                nodes_by_id.insert(id, node);
            }
        }
        let edge_arr = obj
            .get("edges")
            .or_else(|| obj.get("links"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        edges.extend(edge_arr);
        let hyper = obj
            .get("hyperedges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        hyperedges.extend(hyper);
    }
    let mut result = serde_json::Map::new();
    result.insert(
        "nodes".to_string(),
        Value::Array(nodes_by_id.into_values().collect()),
    );
    result.insert("edges".to_string(), Value::Array(edges));
    result.insert("hyperedges".to_string(), Value::Array(hyperedges));
    Ok(Value::Object(result))
}

/// Return the number of nodes in a graph JSON value.
fn count_nodes(graph: &serde_json::Value) -> usize {
    graph
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .map_or(0, std::vec::Vec::len)
}

/// Git merge driver: union-merge `current` and `other` graph JSON files in-place.
///
/// Writes the merged result back into `current`, which git then uses as the
/// resolved merge output. Aborts if the result exceeds `MERGE_MAX_NODES`.
pub(crate) fn cmd_merge_driver(
    _base: &std::path::Path,
    current: &std::path::Path,
    other: &std::path::Path,
) -> Result<()> {
    let cur = read_graph_capped(current)?;
    let oth = read_graph_capped(other)?;
    let merged = merge_two_graphs(cur, oth)?;
    if count_nodes(&merged) > MERGE_MAX_NODES {
        anyhow::bail!("merged graph exceeds {MERGE_MAX_NODES}-node cap; aborting merge");
    }
    let out = serde_json::to_string_pretty(&merged)?;
    std::fs::write(current, out)?;
    Ok(())
}

/// Derive a repo tag from a graph JSON path the same way Python does.
///
/// Python uses `gp.parent.parent.name`: for `<repo>/graphify-out/graph.json`
/// this yields `<repo>`. Files outside that convention fall back to the
/// file stem so callers still get *some* prefix.
fn repo_tag_from_path(graph_path: &std::path::Path) -> String {
    graph_path
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::file_name)
        .and_then(|n| n.to_str())
        .map_or_else(
            || {
                graph_path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repo")
                    .to_string()
            },
            str::to_string,
        )
}

/// Return a unique repo tag per input graph for `merge-graphs` (#1729).
///
/// The naive tag from [`repo_tag_from_path`] (the `graphify-out` parent dir
/// name) is not unique across inputs: `src/graphify-out` and
/// `frontend/src/graphify-out` both yield `src`, so prefixing both node sets
/// with `src::` collides same-stem nodes and silently merges unrelated
/// entities. Colliding tags are widened with their own parent dir
/// (`frontend_src`); any that still collide get an index suffix (`tag-2`) so no
/// two graphs ever share a prefix.
fn distinct_repo_tags(graph_paths: &[std::path::PathBuf]) -> Vec<String> {
    use std::path::Path;
    // `graphify-out/..` → the repo dir (parent of the `graphify-out` dir).
    let repo_dirs: Vec<&Path> = graph_paths
        .iter()
        .map(|p| p.parent().and_then(Path::parent).unwrap_or(Path::new("")))
        .collect();
    let name_of = |d: &Path| -> String {
        d.file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("repo")
            .to_string()
    };
    let mut tags: Vec<String> = repo_dirs.iter().map(|&d| name_of(d)).collect();
    // Widen with the grandparent dir when the bare names collide.
    let distinct = tags.iter().collect::<std::collections::HashSet<_>>().len();
    if distinct != tags.len() {
        tags = repo_dirs
            .iter()
            .map(|&d| {
                let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let parent = d
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if !parent.is_empty() && !name.is_empty() {
                    format!("{parent}_{name}")
                } else if name.is_empty() {
                    "repo".to_string()
                } else {
                    name.to_string()
                }
            })
            .collect();
    }
    // Index-suffix any remaining duplicates so every prefix is distinct. Reserve
    // each returned tag (base or suffixed) and advance the suffix until an UNUSED
    // full tag is found, so a generated `foo-2` can't collide with an input repo
    // literally tagged `foo-2`.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    tags.into_iter()
        .map(|t| {
            if seen.insert(t.clone()) {
                return t;
            }
            let mut n = 2u32;
            loop {
                let candidate = format!("{t}-{n}");
                if seen.insert(candidate.clone()) {
                    return candidate;
                }
                n += 1;
            }
        })
        .collect()
}

/// Prefix every node id (and matching edge endpoints) with `{tag}::` so that
/// cross-repo merges do not collide on shared names like `main` or `init`.
/// Mirrors `graphify_build::prefix_graph_for_global` but operates on the raw
/// JSON value to avoid an extra round-trip through the typed `Graph`.
fn prefix_node_ids(graph_value: &mut serde_json::Value, tag: &str) {
    use serde_json::Value;
    let Some(obj) = graph_value.as_object_mut() else {
        return;
    };
    let prefix = format!("{tag}::");
    if let Some(Value::Array(nodes)) = obj.get_mut("nodes") {
        for node in nodes {
            if let Some(node_obj) = node.as_object_mut()
                && let Some(Value::String(id)) = node_obj.get_mut("id")
            {
                *id = format!("{prefix}{id}");
            }
        }
    }
    for key in ["edges", "links"] {
        if let Some(Value::Array(edges)) = obj.get_mut(key) {
            for edge in edges {
                if let Some(edge_obj) = edge.as_object_mut() {
                    if let Some(Value::String(s)) = edge_obj.get_mut("source") {
                        *s = format!("{prefix}{s}");
                    }
                    if let Some(Value::String(t)) = edge_obj.get_mut("target") {
                        *t = format!("{prefix}{t}");
                    }
                }
            }
        }
    }
}

/// Merge two or more graph JSON files into a single cross-repo graph.
///
/// Each graph's node ids are prefixed with a distinct `<repo>::` tag (via
/// [`distinct_repo_tags`], #1729) before the union-merge, so cross-repo merges
/// never collide even when two inputs share a `graphify-out` parent name. Writes the combined
/// result to `out` (defaulting to `graphify-out/merged-graph.json`).
pub(crate) fn cmd_merge_graphs(
    graphs: &[std::path::PathBuf],
    out: Option<&std::path::Path>,
) -> Result<()> {
    if graphs.len() < 2 {
        anyhow::bail!("merge-graphs requires at least 2 graph files");
    }
    let tags = distinct_repo_tags(graphs);
    // Note (to stderr) when the naive `graphify-out` dir names collide, so the
    // user sees which distinct tags were substituted (#1729).
    let naive: Vec<String> = graphs.iter().map(|g| repo_tag_from_path(g)).collect();
    if naive.iter().collect::<std::collections::HashSet<_>>().len() != naive.len() {
        eprintln!(
            "  note: repo dir names collide; using distinct tags: {}",
            tags.join(", ")
        );
    }
    let mut merged = read_graph_capped(&graphs[0])?;
    prefix_node_ids(&mut merged, &tags[0]);
    for (g, tag) in graphs[1..].iter().zip(&tags[1..]) {
        let mut next = read_graph_capped(g)?;
        prefix_node_ids(&mut next, tag);
        merged = merge_two_graphs(merged, next)?;
    }
    let default_out = graphify_out_dir().join("merged-graph.json");
    let out_path = out.unwrap_or(default_out.as_path());
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(&merged)?;
    std::fs::write(out_path, body)?;
    let n_nodes = count_nodes(&merged);
    let n_edges = merged
        .get("edges")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    println!(
        "Merged {} graphs -> {n_nodes} nodes, {n_edges} edges",
        graphs.len()
    );
    println!("Written to: {}", out_path.display());
    Ok(())
}
