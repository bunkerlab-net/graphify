//! `merge-driver` and `merge-graphs` commands — union-merge graph JSON files.

use anyhow::Result;

use crate::cli::graphify_out_dir;

pub(crate) const MERGE_MAX_BYTES: u64 = 50 * 1024 * 1024;
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
/// Mirrors the Python `_merge_graphs` helper in `__main__.py`.
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

/// Merge two or more graph JSON files into a single cross-repo graph.
///
/// Reads each file in order, union-merging them pairwise. Writes the combined
/// result to `out` (defaulting to `graphify-out/merged-graph.json`).
pub(crate) fn cmd_merge_graphs(
    graphs: &[std::path::PathBuf],
    out: Option<&std::path::Path>,
) -> Result<()> {
    if graphs.len() < 2 {
        anyhow::bail!("merge-graphs requires at least 2 graph files");
    }
    let mut merged = read_graph_capped(&graphs[0])?;
    for g in &graphs[1..] {
        let next = read_graph_capped(g)?;
        merged = merge_two_graphs(merged, next)?;
    }
    let default_out = graphify_out_dir().join("merged-graph.json");
    let out_path = out.unwrap_or(default_out.as_path());
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(&merged)?;
    std::fs::write(out_path, body)?;
    println!("wrote {}", out_path.display());
    Ok(())
}
