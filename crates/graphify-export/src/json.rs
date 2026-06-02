//! JSON export — `to_json`, `prune_dangling_edges`, `backup_if_protected`.
//!
//! Mirrors Python `to_json` / `prune_dangling_edges` / `backup_if_protected`
//! from `graphify-py/graphify/export.py`.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use chrono::Local;
use graphify_build::Graph;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Maximum `graph.json` size (in bytes) eligible for the backup-rate-limit
/// short-circuit. Anything larger forces the normal backup path — we'd rather
/// re-copy than stream-hash a multi-gigabyte file twice on every run.
const MAX_BACKUP_PRELOAD_BYTES: u64 = 512 * 1024 * 1024;

/// Stream-hash `path` via a 64 KiB buffered reader. Returns `None` when the
/// file can't be opened or read.
///
/// The 64 KiB read buffer is heap-allocated to stay under the workspace
/// `clippy::large_stack_arrays` threshold (16 KiB). One allocation per file
/// is negligible against the I/O cost; the same pattern is used in
/// `graphify-detect/src/manifest.rs`.
fn sha256_file(path: &Path) -> Option<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut reader = BufReader::new(File::open(path).ok()?);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize().into())
}

use crate::{
    BACKUP_ARTIFACTS, ExportError, confidence_score, node_community_map, strip_diacritics,
};

/// Attach hyperedges to the graph's metadata.
///
/// Mirrors Python `attach_hyperedges`.
pub fn attach_hyperedges(graph: &mut Graph, hyperedges: &[Value]) {
    // Collect the existing hyperedge IDs first (before taking a mutable borrow).
    let existing_val = graph
        .graph_attrs
        .get("hyperedges")
        .cloned()
        .unwrap_or_else(|| json!([]));

    let seen_ids: IndexSet<String> = existing_val
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|h| h.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut merged: Vec<Value> = existing_val.as_array().cloned().unwrap_or_default();

    for h in hyperedges {
        let hid = h.get("id").and_then(Value::as_str).unwrap_or("");
        if !hid.is_empty() && !seen_ids.contains(hid) {
            merged.push(h.clone());
        }
    }

    graph
        .graph_attrs
        .insert("hyperedges".to_string(), Value::Array(merged));
}

/// Runs `git rev-parse HEAD`, returning `None` if not in a git repo or on failure.
fn git_head() -> Option<String> {
    let r = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if r.status.success() {
        let s = String::from_utf8(r.stdout).ok()?;
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        None
    }
}

/// Export graph to a JSON file in `node_link_data` format.
///
/// Mirrors Python `to_json`. Returns `false` if the write was refused because
/// `force=false` and the new graph would silently shrink the existing one.
///
/// # Errors
///
/// Returns [`ExportError::Io`] or [`ExportError::Json`] on write / serialisation
/// failures.
pub fn to_json(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_path: &Path,
    force: bool,
    built_at_commit: Option<&str>,
) -> Result<bool, ExportError> {
    if !force && would_shrink_graph(graph, output_path) {
        return Ok(false);
    }

    let node_community = node_community_map(communities);
    let nodes = build_node_link_nodes(graph, &node_community);
    let links = build_node_link_edges(graph);
    let hyperedges = graph
        .graph_attrs
        .get("hyperedges")
        .cloned()
        .filter(serde_json::Value::is_array)
        .unwrap_or_else(|| json!([]));

    let mut data = serde_json::Map::new();
    data.insert("directed".to_string(), json!(graph.kind.is_directed()));
    data.insert("multigraph".to_string(), json!(graph.kind.is_multi()));
    data.insert("graph".to_string(), json!({}));
    data.insert("nodes".to_string(), Value::Array(nodes));
    data.insert("links".to_string(), Value::Array(links));
    data.insert("hyperedges".to_string(), hyperedges);

    let commit = built_at_commit.map(str::to_string).or_else(git_head);
    if let Some(c) = commit {
        data.insert("built_at_commit".to_string(), Value::String(c));
    }

    let json_text = serde_json::to_string_pretty(&Value::Object(data))?;
    std::fs::write(output_path, json_text)?;
    Ok(true)
}

/// Safety check: refuse to silently shrink an existing graph (#479). Returns
/// `true` if `graph` would shrink the on-disk version at `output_path`.
fn would_shrink_graph(graph: &Graph, output_path: &Path) -> bool {
    if !output_path.exists() {
        return false;
    }
    // Reject oversized existing files before reading them into memory. Fail
    // *closed*: if the existing file blows the memory-bomb cap, refuse the
    // overwrite (treat it as a shrink). Otherwise an attacker could plant a
    // 600 MiB graph.json and we'd silently overwrite it with a normal one.
    if graphify_security::check_graph_file_size_cap(output_path).is_err() {
        eprintln!(
            "[graphify] WARNING: existing graph.json at {} exceeds the \
             memory-bomb size cap and cannot be inspected for shrink \
             protection. Refusing to overwrite. Pass force=True to override.",
            output_path.display()
        );
        return true;
    }
    let Ok(text) = std::fs::read_to_string(output_path) else {
        return false;
    };
    let Ok(existing_data) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let existing_n = existing_data
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let new_n = graph.node_count();
    if new_n < existing_n {
        eprintln!(
            "[graphify] WARNING: new graph has {new_n} nodes but existing \
             graph.json has {existing_n}. Refusing to overwrite — you may be \
             missing chunk files from a previous session. \
             Pass force=True to override."
        );
        return true;
    }
    false
}

/// Build the `node_link_data.nodes` array (with Python field-order parity).
fn build_node_link_nodes(graph: &Graph, node_community: &IndexMap<String, i64>) -> Vec<Value> {
    let mut nodes: Vec<Value> = graph
        .nodes()
        .map(|(id, attrs)| {
            let mut node = IndexMap::new();
            for (k, v) in attrs {
                node.insert(k.clone(), v.clone());
            }
            node.insert(
                "community".to_string(),
                node_community
                    .get(id)
                    .map_or(Value::Null, |&cid| json!(cid)),
            );
            let label = attrs
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(id.as_str());
            node.insert(
                "norm_label".to_string(),
                Value::String(strip_diacritics(label).to_lowercase()),
            );
            node.insert("id".to_string(), Value::String(id.clone()));
            Value::Object(node.into_iter().collect())
        })
        .collect();

    // Python node_link_data emits: <all node attrs in insertion order>, id, community,
    // norm_label. Reorder each node's tail to match.
    for node in &mut nodes {
        if let Value::Object(map) = node {
            let id_val = map.remove("id");
            let community_val = map.remove("community");
            let norm_label_val = map.remove("norm_label");
            if let Some(v) = id_val {
                map.insert("id".to_string(), v);
            }
            if let Some(v) = community_val {
                map.insert("community".to_string(), v);
            }
            if let Some(v) = norm_label_val {
                map.insert("norm_label".to_string(), v);
            }
        }
    }
    nodes
}

/// Build the `node_link_data.links` array, restoring true source/target from `_src`/`_tgt`.
fn build_node_link_edges(graph: &Graph) -> Vec<Value> {
    let mut links: Vec<Value> = Vec::new();
    for edge in graph.edges() {
        let mut link = IndexMap::new();
        for (k, v) in &edge.attrs {
            link.insert(k.clone(), v.clone());
        }
        if !link.contains_key("confidence_score") {
            let conf = link
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("EXTRACTED");
            link.insert(
                "confidence_score".to_string(),
                json!(confidence_score(conf)),
            );
        }
        let true_src = link.get("_src").and_then(Value::as_str).map(str::to_string);
        let true_tgt = link.get("_tgt").and_then(Value::as_str).map(str::to_string);
        link.shift_remove("_src");
        link.shift_remove("_tgt");
        let source = true_src.unwrap_or_else(|| edge.source.clone());
        let target = true_tgt.unwrap_or_else(|| edge.target.clone());
        link.insert("source".to_string(), Value::String(source));
        link.insert("target".to_string(), Value::String(target));
        links.push(Value::Object(link.into_iter().collect()));
    }
    links
}

/// Remove edges whose source or target node is not in the node set.
///
/// Returns the cleaned graph data and the number of pruned edges.
///
/// Mirrors Python `prune_dangling_edges`.
#[must_use]
pub fn prune_dangling_edges(mut graph_data: Value) -> (Value, usize) {
    let node_ids: IndexSet<String> = graph_data
        .get("nodes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let links_key = if graph_data.get("links").is_some() {
        "links"
    } else {
        "edges"
    };

    let before = graph_data
        .get(links_key)
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    if let Value::Object(map) = &mut graph_data
        && let Some(Value::Array(edges)) = map.get_mut(links_key)
    {
        edges.retain(|e| {
            let src = e.get("source").and_then(Value::as_str).unwrap_or("");
            let tgt = e.get("target").and_then(Value::as_str).unwrap_or("");
            node_ids.contains(src) && node_ids.contains(tgt)
        });
    }

    let after = graph_data
        .get(links_key)
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    (graph_data, before - after)
}

/// Snapshot graph artifacts to a dated subfolder before an overwrite.
///
/// Triggers when `graph.json` exists AND either:
/// - `.graphify_semantic_marker` is present, or
/// - `.graphify_labels.json` contains at least one non-default community label.
///
/// Returns the backup folder path, or `None` if no backup was taken.
/// Never fails — backup failure prints a warning and returns `None`.
/// Set `GRAPHIFY_NO_BACKUP=1` to disable.
///
/// Same-day rate-limiting: if today's backup folder already exists and its
/// `graph.json` content is byte-identical (sha256 match) to the source,
/// returns the existing folder without re-copying. If content has changed,
/// the existing folder is overwritten in place — one folder per day, always
/// the latest pre-overwrite state.
///
/// Mirrors Python `backup_if_protected`.
#[must_use]
pub fn backup_if_protected(out_dir: &Path) -> Option<PathBuf> {
    if std::env::var("GRAPHIFY_NO_BACKUP").is_ok_and(|v| !v.is_empty()) {
        return None;
    }
    let graph_src = out_dir.join("graph.json");
    if !graph_src.exists() {
        return None;
    }

    let is_semantic = out_dir.join(".graphify_semantic_marker").exists();
    let mut is_curated = false;
    let labels_file = out_dir.join(".graphify_labels.json");
    if labels_file.exists()
        && let Ok(text) = std::fs::read_to_string(&labels_file)
        && let Ok(labels) = serde_json::from_str::<Value>(&text)
        && let Some(obj) = labels.as_object()
    {
        is_curated = obj
            .iter()
            .any(|(k, v)| v.as_str() != Some(&format!("Community {k}")));
    }

    if !is_semantic && !is_curated {
        return None;
    }

    let today = Local::now().format("%Y-%m-%d").to_string();
    let backup_dir = out_dir.join(&today);
    let backup_graph = backup_dir.join("graph.json");

    // Short-circuit: if today's backup already has identical graph.json
    // content, nothing to do. Mirrors the Python `if src_hash == bak_hash`
    // guard added in graphify-py 3efae38.
    //
    // Streaming sha256 (64 KiB chunks) keeps the comparison memory-bounded
    // regardless of graph size, and a size-equality preflight catches the
    // common "different bytes" case in O(1). Files exceeding
    // `MAX_BACKUP_PRELOAD_BYTES` skip the short-circuit entirely — the user
    // gets the normal backup path rather than paying for a full re-hash on
    // every invocation.
    if backup_dir.exists() && backup_graph.exists() {
        let src_meta = std::fs::metadata(&graph_src).ok();
        let bak_meta = std::fs::metadata(&backup_graph).ok();
        if let (Some(s), Some(b)) = (src_meta, bak_meta)
            && s.len() == b.len()
            && s.len() <= MAX_BACKUP_PRELOAD_BYTES
            && let (Some(src_hash), Some(bak_hash)) =
                (sha256_file(&graph_src), sha256_file(&backup_graph))
            && src_hash == bak_hash
        {
            return Some(backup_dir);
        }
    }

    let mut reasons = Vec::new();
    if is_semantic {
        reasons.push("semantic");
    }
    if is_curated {
        reasons.push("curated");
    }
    let reason = reasons.join("+");

    match try_backup(out_dir, &backup_dir, &reason) {
        Ok(Some(path)) => Some(path),
        Ok(None) => None,
        Err(e) => {
            eprintln!("[graphify] warning: backup failed ({e}) - continuing with overwrite");
            None
        }
    }
}

/// Copy `BACKUP_ARTIFACTS` from `out_dir` to `backup_dir`, returning the destination
/// path if at least one file was copied.
fn try_backup(
    out_dir: &Path,
    backup_dir: &Path,
    reason: &str,
) -> Result<Option<PathBuf>, ExportError> {
    std::fs::create_dir_all(backup_dir)?;
    let mut copied = 0_usize;
    for name in BACKUP_ARTIFACTS {
        let src = out_dir.join(name);
        if src.exists() {
            // Best-effort per-file — ignore individual failures
            if std::fs::copy(&src, backup_dir.join(name)).is_ok() {
                copied += 1;
            }
        }
    }
    if copied > 0 {
        let dir_name = backup_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("backup");
        println!("[graphify] backed up {reason} graph ({copied} files) -> {dir_name}/");
        Ok(Some(backup_dir.to_path_buf()))
    } else {
        Ok(None)
    }
}
