//! Convert a raw extraction `serde_json::Value` into nodes and edges on a
//! [`Graph`].

use indexmap::{IndexMap, IndexSet};
use rayon::prelude::*;
use serde_json::Value;

use crate::file_type::coerce_file_type;
use crate::graph::Graph;
use crate::normalize::{norm_source_file, normalize_id};

/// Edge-count threshold above which per-edge resolution is dispatched to Rayon.
const PARALLEL_EDGE_THRESHOLD: usize = 1024;

/// Language family for the cross-language `calls` INFERRED filter.
///
/// Mirrors the per-extension table in `build.py` `build_from_json`: when
/// both endpoints of an `INFERRED` `calls` edge resolve to different
/// language families, the edge is dropped because shared short names
/// (`render`, `parse`, ...) produce phantom call edges across language
/// boundaries in multi-language chunks.
// Callers (`source_file_ext`) always pass an already-lowercased extension,
// so the match arms only need to enumerate the lowercase forms. Skipping the
// `to_ascii_lowercase` call avoids the per-call `String` allocation on a hot
// loop over every edge.
#[must_use]
fn language_family(ext: &str) -> Option<&'static str> {
    match ext {
        "py" | "pyi" => Some("py"),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Some("js"),
        "go" => Some("go"),
        "rs" => Some("rs"),
        "java" | "kt" | "scala" | "groovy" => Some("jvm"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "hpp" => Some("cpp"),
        "rb" => Some("rb"),
        "php" => Some("php"),
        "cs" => Some("cs"),
        "swift" => Some("swift"),
        "lua" => Some("lua"),
        _ => None,
    }
}

/// Return the extension (without the leading dot, lowercased) of a node's
/// `source_file`, or an empty string if absent or extensionless.
#[must_use]
fn source_file_ext(node_source_files: &IndexMap<String, String>, id: &str) -> String {
    node_source_files
        .get(id)
        .map(std::path::Path::new)
        .and_then(std::path::Path::extension)
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Normalise node objects inside an extraction dict in place.
///
/// Renames `source` → `source_file` and coerces `file_type` values to
/// one of the canonical strings (see [`crate::file_type`]).
pub(crate) fn canonicalise_nodes(extraction: &mut Value) {
    let Some(nodes) = extraction
        .as_object_mut()
        .and_then(|o| o.get_mut("nodes"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for node in nodes.iter_mut() {
        let Some(map) = node.as_object_mut() else {
            continue;
        };
        if map.contains_key("source") && !map.contains_key("source_file") {
            let src = map.remove("source").unwrap_or(Value::Null);
            map.insert("source_file".to_string(), src);
        }
        if let Some(new_ft) = coerce_file_type(map.get("file_type")) {
            map.insert("file_type".to_string(), Value::String(new_ft));
        }
    }
}

/// Insert all nodes from an extraction dict into `graph`, normalising
/// `source_file` paths relative to `root_str` if provided.
pub(crate) fn add_nodes(graph: &mut Graph, extraction: &mut Value, root_str: Option<&str>) {
    let Some(nodes) = extraction
        .as_object_mut()
        .and_then(|o| o.get_mut("nodes"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for node in nodes.iter_mut() {
        let Some(map) = node.as_object_mut() else {
            continue;
        };
        let Some(id) = map.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(Value::String(sf)) = map.get_mut("source_file") {
            *sf = norm_source_file(sf, root_str);
        }
        let mut attrs: IndexMap<String, Value> = IndexMap::new();
        for (k, v) in &*map {
            if k == "id" {
                continue;
            }
            attrs.insert(k.clone(), v.clone());
        }
        graph.add_node(&id, attrs);
    }
}

/// Resolve a raw edge endpoint string to an exact node ID, falling back
/// to the normalised-ID lookup table when the raw string does not match
/// any existing node verbatim.
fn resolve_edge_id(
    raw: &str,
    node_ids: &IndexSet<String>,
    norm_to_id: &IndexMap<String, String>,
) -> String {
    if node_ids.contains(raw) {
        return raw.to_string();
    }
    norm_to_id
        .get(&normalize_id(raw))
        .cloned()
        .unwrap_or_else(|| raw.to_string())
}

/// Insert all edges from an extraction dict into `graph`, resolving
/// endpoint IDs and normalising `source_file` paths.
///
/// Edges whose endpoints cannot be resolved to existing nodes are
/// dropped — this matches the Python reference behaviour for dangling
/// edges (stdlib / external imports).
///
/// The previous implementation cloned the entire edge `Map` for every
/// edge (to support a rare `from`/`to` rename). On a 36k-edge graph that
/// dominated `build_from_json`. The optimised path borrows the source map
/// when the canonical `source`/`target` keys are already present, falling
/// back to the clone path only for legacy `from`/`to` inputs.
pub(crate) fn add_edges(graph: &mut Graph, extraction: &Value, root_str: Option<&str>) {
    let Some(edges) = extraction
        .as_object()
        .and_then(|o| o.get("edges"))
        .and_then(Value::as_array)
    else {
        return;
    };
    let node_ids: IndexSet<String> = graph.nodes().map(|(id, _)| id.clone()).collect();
    let norm_to_id: IndexMap<String, String> = node_ids
        .iter()
        .map(|nid| (normalize_id(nid), nid.clone()))
        .collect();
    // Snapshot each node's `source_file` so the cross-language `calls`
    // INFERRED filter can resolve language families without re-borrowing
    // `graph` from inside the per-edge closure.
    let node_source_files: IndexMap<String, String> = graph
        .nodes()
        .map(|(id, attrs)| {
            (
                id.clone(),
                attrs
                    .get("source_file")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect();

    // Per-edge resolution is pure read-only work over `node_ids` and
    // `norm_to_id` — fan out across Rayon. We collect the resolved
    // `(src, tgt, attrs)` tuples and then perform the actual graph
    // mutation in a single sequential pass below to preserve edge
    // insertion order.
    let resolve_edge = |edge: &Value| -> Option<(String, String, IndexMap<String, Value>)> {
        let orig = edge.as_object()?;
        let has_canonical = orig.contains_key("source") && orig.contains_key("target");
        let (src_str, tgt_str, source_map);
        let canonical_map: &serde_json::Map<String, Value> = if has_canonical {
            src_str = orig.get("source").and_then(Value::as_str)?;
            tgt_str = orig.get("target").and_then(Value::as_str)?;
            orig
        } else {
            let mut map = orig.clone();
            if !map.contains_key("source")
                && let Some(v) = map.remove("from")
            {
                map.insert("source".to_string(), v);
            }
            if !map.contains_key("target")
                && let Some(v) = map.remove("to")
            {
                map.insert("target".to_string(), v);
            }
            source_map = map;
            src_str = source_map.get("source").and_then(Value::as_str)?;
            tgt_str = source_map.get("target").and_then(Value::as_str)?;
            &source_map
        };

        let resolved_src = resolve_edge_id(src_str, &node_ids, &norm_to_id);
        let resolved_tgt = resolve_edge_id(tgt_str, &node_ids, &norm_to_id);
        if !node_ids.contains(&resolved_src) || !node_ids.contains(&resolved_tgt) {
            return None;
        }
        // Drop cross-language INFERRED `calls` edges — same short names
        // (`render`, `parse`, ...) appear across language boundaries in
        // multi-language chunks, producing phantom edges that don't
        // represent real call relationships. Mirrors the dispatcher added
        // in graphify-py `build.py` `build_from_json`.
        let relation = canonical_map.get("relation").and_then(Value::as_str);
        let confidence = canonical_map.get("confidence").and_then(Value::as_str);
        if relation == Some("calls") && confidence == Some("INFERRED") {
            let src_ext = source_file_ext(&node_source_files, &resolved_src);
            let tgt_ext = source_file_ext(&node_source_files, &resolved_tgt);
            if !src_ext.is_empty()
                && !tgt_ext.is_empty()
                && language_family(&src_ext) != language_family(&tgt_ext)
            {
                return None;
            }
        }
        let mut attrs: IndexMap<String, Value> = IndexMap::new();
        for (k, v) in canonical_map {
            if k == "source" || k == "target" {
                continue;
            }
            if k == "source_file"
                && let Value::String(sf) = v
            {
                attrs.insert(k.clone(), Value::String(norm_source_file(sf, root_str)));
                continue;
            }
            attrs.insert(k.clone(), v.clone());
        }
        attrs.insert("_src".to_string(), Value::String(resolved_src.clone()));
        attrs.insert("_tgt".to_string(), Value::String(resolved_tgt.clone()));
        Some((resolved_src, resolved_tgt, attrs))
    };

    let resolved: Vec<(String, String, IndexMap<String, Value>)> =
        if edges.len() >= PARALLEL_EDGE_THRESHOLD {
            edges.par_iter().filter_map(resolve_edge).collect()
        } else {
            edges.iter().filter_map(resolve_edge).collect()
        };

    // Bulk insert — O(N + E) instead of the O(N²) shape that
    // per-call `add_edge` would produce when scanning for duplicates.
    graph.bulk_add_edges(resolved);
}
