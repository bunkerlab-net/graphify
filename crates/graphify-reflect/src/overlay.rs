//! Work-memory overlay sidecar (`.graphify_learning.json`).
//!
//! A derived, experiential projection of the reflect aggregate, written next to
//! `graph.json`. It carries which nodes have proven preferred / tentative /
//! contested, a code fingerprint for staleness detection, and a short provenance
//! trail. `graph.json` (durable structural truth) is never touched — read
//! surfaces merge this overlay in only at display time.
//!
//! Ports the work-memory overlay from `graphify-py/graphify/reflect.py` (#1441).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::aggregate::AggResult;

/// Sidecar filename written next to `graph.json`.
pub const LEARNING_SIDECAR_NAME: &str = ".graphify_learning.json";
/// Schema version stamped into the sidecar.
pub const LEARNING_SCHEMA_VERSION: u64 = 1;
/// Most-recent provenance entries kept per node.
const PROVENANCE_CAP: usize = 5;

/// id→id, label→[ids], id→node maps from `graph.json`. Best-effort; an
/// unreadable/garbage graph yields empty maps.
/// `(id_set, label→[ids], id→node)` extracted from `graph.json` for citation
/// resolution.
type GraphMaps = (
    HashSet<String>,
    IndexMap<String, Vec<String>>,
    IndexMap<String, Value>,
);

fn build_id_label_maps(graph_path: &Path) -> GraphMaps {
    let mut id_set: HashSet<String> = HashSet::new();
    let mut label_to_ids: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut node_by_id: IndexMap<String, Value> = IndexMap::new();
    let Ok(text) = std::fs::read_to_string(graph_path) else {
        return (id_set, label_to_ids, node_by_id);
    };
    let Ok(data) = serde_json::from_str::<Value>(&text) else {
        return (id_set, label_to_ids, node_by_id);
    };
    let Some(nodes) = data.get("nodes").and_then(Value::as_array) else {
        return (id_set, label_to_ids, node_by_id);
    };
    for n in nodes {
        let Some(nid) = n.get("id").and_then(Value::as_str) else {
            continue;
        };
        id_set.insert(nid.to_string());
        node_by_id.insert(nid.to_string(), n.clone());
        if let Some(label) = n.get("label").and_then(Value::as_str) {
            label_to_ids
                .entry(label.to_string())
                .or_default()
                .push(nid.to_string());
        }
    }
    (id_set, label_to_ids, node_by_id)
}

/// Resolve a cited node (a label OR an id) to a single canonical node id.
/// `None` when unresolved (gone) or ambiguous (label shared by >1 id).
fn resolve_canonical_id(
    cited: &str,
    id_set: &HashSet<String>,
    label_to_ids: &IndexMap<String, Vec<String>>,
) -> Option<String> {
    if id_set.contains(cited) {
        return Some(cited.to_string());
    }
    match label_to_ids.get(cited) {
        Some(ids) if ids.len() == 1 => Some(ids[0].clone()),
        _ => None,
    }
}

/// Locate a node's `source_file` on disk, returning an existing file or `None`.
///
/// `source_file` is stored relative to the PROJECT root, but `graph.json` may
/// live in `<root>/graphify-out/` or directly at the root. Resolve the root in
/// the most-likely order (`.graphify_root` marker, layout-appropriate root, the
/// other, cwd) and return the first candidate that exists. The same search runs
/// at write and read time so writer and reader resolve to the same file.
fn resolve_source_path(src: &str, graph_path: &Path) -> Option<PathBuf> {
    if src.is_empty() {
        return None;
    }
    let p = Path::new(src);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let out_dir = graph_path.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(recorded) = std::fs::read_to_string(out_dir.join(".graphify_root")) {
        let recorded = recorded.trim();
        if !recorded.is_empty() {
            candidates.push(PathBuf::from(recorded));
        }
    }
    let is_out_layout = out_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == graphify_security::graphify_out_name());
    let parent = out_dir.parent().unwrap_or(out_dir);
    if is_out_layout {
        candidates.push(parent.to_path_buf());
        candidates.push(out_dir.to_path_buf());
    } else {
        candidates.push(out_dir.to_path_buf());
        candidates.push(parent.to_path_buf());
    }
    candidates.push(PathBuf::from("."));
    let mut seen: HashSet<String> = HashSet::new();
    for base in candidates {
        if !seen.insert(base.to_string_lossy().into_owned()) {
            continue;
        }
        let cand = base.join(p);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// SHA256 of file CONTENT only (no path mixed in), so the fingerprint is
/// independent of which root resolved the file. `""` on read failure.
fn content_hash(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut h = Sha256::new();
            h.update(&bytes);
            hex::encode(h.finalize())
        }
        Err(_) => String::new(),
    }
}

/// Content hash of the node's `source_file`, or `""` if unavailable. Coarse on
/// purpose (file-level) — over-flags rather than under-flags, the safe direction.
fn code_fingerprint(node: Option<&Value>, graph_path: &Path) -> String {
    let Some(node) = node else {
        return String::new();
    };
    let src = node
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("");
    match resolve_source_path(src, graph_path) {
        Some(sp) => content_hash(&sp),
        None => String::new(),
    }
}

/// Most-recent-first, capped provenance entries for a node.
fn provenance_for(
    node: &str,
    prov_map: &IndexMap<String, Vec<(String, String, String)>>,
) -> Vec<Value> {
    let Some(events) = prov_map.get(node) else {
        return Vec::new();
    };
    let mut ordered = events.clone();
    // (date desc, then question) for a stable tiebreak.
    ordered.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    ordered
        .into_iter()
        .take(PROVENANCE_CAP)
        .map(|(date, question, outcome)| {
            // Sorted keys (date, outcome, q) for byte-stable output.
            let mut m = Map::new();
            m.insert("date".to_string(), Value::String(date));
            m.insert("outcome".to_string(), Value::String(outcome));
            m.insert("q".to_string(), Value::String(question));
            Value::Object(m)
        })
        .collect()
}

/// A resolved-and-scored overlay entry before it is emitted as sorted JSON.
struct OverlayEntry {
    node: String,
    status: &'static str,
    score: f64,
    uses: usize,
    last: String,
    verdict: Option<String>,
    neg: Option<usize>,
}

/// Project the reflect aggregate into the sidecar `{version, generated_at,
/// nodes}` structure, keyed by canonical node id. Built from preferred +
/// tentative + contested (not dead ends). Citations that don't resolve to
/// exactly one node id are skipped. Node ids and entry keys are emitted in
/// sorted order for byte-stable output.
#[must_use]
pub fn build_learning_overlay(agg: &AggResult, graph_path: &Path, now: DateTime<Utc>) -> Value {
    let (id_set, label_to_ids, node_by_id) = build_id_label_maps(graph_path);
    let prov_map = &agg.node_provenance;

    // Preferred > tentative > contested: first status wins for a canonical id.
    let mut candidates: Vec<OverlayEntry> = Vec::new();
    for e in &agg.preferred {
        candidates.push(OverlayEntry {
            node: e.node.clone(),
            status: "preferred",
            score: e.score,
            uses: e.n,
            last: String::new(),
            verdict: None,
            neg: None,
        });
    }
    for e in &agg.tentative {
        candidates.push(OverlayEntry {
            node: e.node.clone(),
            status: "tentative",
            score: e.score,
            uses: e.n,
            last: String::new(),
            verdict: None,
            neg: None,
        });
    }
    for e in &agg.contested {
        candidates.push(OverlayEntry {
            node: e.node.clone(),
            status: "contested",
            score: e.score,
            uses: e.pos,
            last: e.last.clone(),
            verdict: Some(e.verdict.clone()),
            neg: Some(e.neg),
        });
    }

    let mut nodes_out: IndexMap<String, Value> = IndexMap::new();
    for c in candidates {
        let Some(cid) = resolve_canonical_id(&c.node, &id_set, &label_to_ids) else {
            continue; // ambiguous or stale — can't display against a single node
        };
        if nodes_out.contains_key(&cid) {
            continue; // first status wins
        }
        let node = node_by_id.get(&cid);
        let label = node
            .and_then(|n| n.get("label").and_then(Value::as_str))
            .unwrap_or(&c.node)
            .to_string();
        let source_file = node
            .and_then(|n| n.get("source_file").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let provenance = provenance_for(&c.node, prov_map);
        // preferred/tentative carry no verdict; derive `last` from provenance
        // when the finalizer didn't record one (positive-only buckets).
        let last = if c.last.is_empty() && c.status != "contested" {
            provenance
                .first()
                .and_then(|p| p.get("date").and_then(Value::as_str))
                .unwrap_or("")
                .to_string()
        } else {
            c.last
        };
        // Insert keys in sorted (alphabetical) order for byte-stable output.
        let mut entry = Map::new();
        entry.insert(
            "code_fingerprint".to_string(),
            Value::String(code_fingerprint(node, graph_path)),
        );
        entry.insert("label".to_string(), Value::String(label));
        entry.insert("last".to_string(), Value::String(last));
        if let Some(neg) = c.neg {
            entry.insert("neg".to_string(), json!(neg));
        }
        entry.insert("provenance".to_string(), Value::Array(provenance));
        entry.insert("score".to_string(), json!(c.score));
        entry.insert("source_file".to_string(), Value::String(source_file));
        entry.insert("status".to_string(), Value::String(c.status.to_string()));
        entry.insert("uses".to_string(), json!(c.uses));
        if let Some(verdict) = c.verdict {
            entry.insert("verdict".to_string(), Value::String(verdict));
        }
        nodes_out.insert(cid, Value::Object(entry));
    }

    // Emit node ids in sorted order for byte-stable output.
    let mut sorted_ids: Vec<&String> = nodes_out.keys().collect();
    sorted_ids.sort();
    let mut nodes_map = Map::new();
    for id in sorted_ids {
        nodes_map.insert(id.clone(), nodes_out[id].clone());
    }

    let mut top = Map::new();
    top.insert(
        "generated_at".to_string(),
        // `to_rfc3339` matches Python `datetime.isoformat()` for an aware UTC time
        // (`...+00:00`), keeping the sidecar deterministic across runs.
        Value::String(now.to_rfc3339()),
    );
    top.insert("nodes".to_string(), Value::Object(nodes_map));
    top.insert("version".to_string(), json!(LEARNING_SCHEMA_VERSION));
    Value::Object(top)
}

/// Write `.graphify_learning.json` next to `graph_path` deterministically.
/// Returns the sidecar path.
///
/// # Errors
///
/// Returns [`std::io::Error`] if the sidecar cannot be written.
pub fn write_learning_sidecar(
    agg: &AggResult,
    graph_path: &Path,
    now: DateTime<Utc>,
) -> std::io::Result<PathBuf> {
    let overlay = build_learning_overlay(agg, graph_path, now);
    let sidecar = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LEARNING_SIDECAR_NAME);
    let body = serde_json::to_string_pretty(&overlay).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(&sidecar, format!("{body}\n"))?;
    Ok(sidecar)
}

/// Load the sidecar next to `graph_path`, returning `{node_id -> entry}` with a
/// recomputed `stale: bool` per entry. Best-effort → empty map on any error.
#[must_use]
pub fn load_learning_overlay(graph_path: &Path) -> IndexMap<String, Value> {
    let sidecar = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LEARNING_SIDECAR_NAME);
    let mut out: IndexMap<String, Value> = IndexMap::new();
    let Ok(text) = std::fs::read_to_string(&sidecar) else {
        return out;
    };
    let Ok(data) = serde_json::from_str::<Value>(&text) else {
        return out;
    };
    let Some(nodes) = data.get("nodes").and_then(Value::as_object) else {
        return out;
    };
    for (nid, entry) in nodes {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let mut merged = obj.clone();
        merged.insert("stale".to_string(), Value::Bool(is_stale(obj, graph_path)));
        out.insert(nid.clone(), Value::Object(merged));
    }
    out
}

/// `true` if the node's source file changed (or vanished) since the fingerprint
/// was taken. Uses the same resolution + content hash as the writer.
fn is_stale(entry: &Map<String, Value>, graph_path: &Path) -> bool {
    let src = entry
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("");
    if src.is_empty() {
        return false; // no file to track — nothing to re-verify
    }
    let Some(sp) = resolve_source_path(src, graph_path) else {
        return true; // file gone / unfindable — re-verify
    };
    let stored = entry
        .get("code_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("");
    if stored.is_empty() {
        return true; // had a file but never fingerprinted it — can't trust
    }
    content_hash(&sp) != stored
}
