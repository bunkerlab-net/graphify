//! `merge-chunks` and `merge-semantic` commands — merge extraction JSON chunk
//! files and cached semantic results.

use std::path::PathBuf;

use anyhow::Result;

/// Merge multiple extraction JSON chunk files into one.
///
/// Concatenates `{nodes, edges, hyperedges, input_tokens, output_tokens}`
/// from each chunk, deduplicating nodes by `id` (first writer wins).
/// Mirrors `graphify merge-chunks` at `__main__.py:2952`.
pub(crate) fn cmd_merge_chunks(chunks: &[PathBuf], out: &std::path::Path) -> Result<()> {
    use serde_json::Value;
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();
    let mut hyperedges: Vec<Value> = Vec::new();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut seen_ids: indexmap::IndexSet<String> = indexmap::IndexSet::new();

    for chunk_path in chunks {
        let raw = match std::fs::read_to_string(chunk_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "[graphify merge-chunks] warning: skipping {}: {e}",
                    chunk_path.display()
                );
                continue;
            }
        };
        let chunk: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "[graphify merge-chunks] warning: skipping {}: {e}",
                    chunk_path.display()
                );
                continue;
            }
        };
        if let Some(Value::Array(ns)) = chunk.get("nodes") {
            for n in ns {
                let id = n
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !id.is_empty() && seen_ids.insert(id) {
                    nodes.push(n.clone());
                }
            }
        }
        if let Some(Value::Array(es)) = chunk.get("edges") {
            edges.extend(es.iter().cloned());
        }
        if let Some(Value::Array(hs)) = chunk.get("hyperedges") {
            hyperedges.extend(hs.iter().cloned());
        }
        input_tokens += chunk
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        output_tokens += chunk
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let node_count = nodes.len();
    let edge_count = edges.len();
    let chunk_count = chunks.len();
    let merged = serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "hyperedges": hyperedges,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
    });
    std::fs::write(out, serde_json::to_string(&merged)?)?;
    println!(
        "Merged {chunk_count} chunks: {node_count} nodes, {edge_count} edges, \
         {input_tokens} in / {output_tokens} out tokens",
    );
    Ok(())
}

/// Merge cached semantic results with fresh extraction output.
///
/// Cached entries take priority over new ones on node-id collision.
/// Mirrors `graphify merge-semantic` at `__main__.py:3000`.
pub(crate) fn cmd_merge_semantic(
    cached: Option<&std::path::Path>,
    new: Option<&std::path::Path>,
    out: &std::path::Path,
) -> Result<()> {
    use serde_json::Value;

    /// Load a JSON file if it exists; return an empty object on absence.
    fn load_opt(path: Option<&std::path::Path>) -> Result<Value> {
        let Some(p) = path else {
            return Ok(serde_json::json!({"nodes":[],"edges":[],"hyperedges":[]}));
        };
        if !p.exists() {
            return Ok(serde_json::json!({"nodes":[],"edges":[],"hyperedges":[]}));
        }
        let raw = std::fs::read_to_string(p)?;
        Ok(serde_json::from_str(&raw)?)
    }

    let cached_data = load_opt(cached)?;
    let new_data = load_opt(new)?;

    // Cached entries win: iterate cached first, then new.
    let mut seen_ids: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    let mut all_nodes: Vec<Value> = Vec::new();
    let empty_arr: Vec<Value> = vec![];
    let cached_nodes = cached_data
        .get("nodes")
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);
    let new_nodes = new_data
        .get("nodes")
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);
    for n in cached_nodes.iter().chain(new_nodes.iter()) {
        let id = n
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !id.is_empty() && seen_ids.insert(id) {
            all_nodes.push(n.clone());
        }
    }

    let mut all_edges: Vec<Value> = Vec::new();
    for src in [&cached_data, &new_data] {
        if let Some(Value::Array(es)) = src.get("edges") {
            all_edges.extend(es.iter().cloned());
        }
    }
    let mut all_hyperedges: Vec<Value> = Vec::new();
    for src in [&cached_data, &new_data] {
        if let Some(Value::Array(hs)) = src.get("hyperedges") {
            all_hyperedges.extend(hs.iter().cloned());
        }
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let node_count = all_nodes.len();
    let edge_count = all_edges.len();
    let merged = serde_json::json!({
        "nodes": all_nodes,
        "edges": all_edges,
        "hyperedges": all_hyperedges,
    });
    std::fs::write(out, serde_json::to_string(&merged)?)?;
    println!("Merged: {node_count} nodes, {edge_count} edges");
    Ok(())
}
