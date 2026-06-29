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
        "cc" | "cpp" | "hpp" | "cu" | "cuh" | "metal" => Some("cpp"),
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
/// Deterministic sort key for an edge: `(source|from, target|to, relation)`.
///
/// String values are used verbatim; missing keys sort as empty strings. Mirrors
/// the `sorted(..., key=...)` call in graphify-py `build_from_json`.
#[must_use]
fn edge_sort_key(edge: &Value) -> (String, String, String) {
    let obj = edge.as_object();
    let field = |primary: &str, fallback: &str| -> String {
        obj.and_then(|o| o.get(primary).or_else(|| o.get(fallback)))
            .map(value_to_sort_string)
            .unwrap_or_default()
    };
    let relation = obj
        .and_then(|o| o.get("relation"))
        .map(value_to_sort_string)
        .unwrap_or_default();
    (field("source", "from"), field("target", "to"), relation)
}

/// Stringify a JSON value for sort comparison: strings verbatim, null → empty,
/// anything else via its JSON representation.
#[must_use]
fn value_to_sort_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `(basename, label)` key for a node, or `None` when either is empty.
fn ghost_key(attrs: &IndexMap<String, Value>) -> Option<(String, String)> {
    let label = attrs
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if label.is_empty() {
        return None;
    }
    let sf = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("");
    if sf.is_empty() {
        return None;
    }
    let basename = std::path::Path::new(sf)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if basename.is_empty() {
        return None;
    }
    Some((basename.to_string(), label.to_string()))
}

/// Python-style truthiness for the `source_location` canonical signal.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_none_or(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Merge LLM ghost-duplicate nodes into their AST canonical nodes (#1145,
/// extended #1271).
///
/// AST extraction uses parent-qualified IDs (`mingpt_bpe_get_pairs`) while the
/// LLM emits bare-stem IDs (`bpe_get_pairs`) for the same symbol. Canonical
/// nodes are those stamped `_origin == "ast"` (AST always wins) or carrying a
/// `source_location`; any non-AST node sharing `(basename, label)` with a
/// canonical node is a ghost. Ghost nodes are removed from `graph` and a
/// `ghost_id -> canonical_id` remap is returned so [`add_edges`] can re-point
/// their edges.
pub(crate) fn merge_ghost_duplicates(graph: &mut Graph) -> IndexMap<String, String> {
    // Pass 1: collect canonical nodes — AST-origin nodes take precedence over
    // LLM nodes; among non-AST nodes the first occurrence per key wins. When 2+
    // AST nodes share a key (same-named symbols in same-named files across
    // directories, e.g. `render` in two `index.ts`), the key is ambiguous:
    // merging a ghost onto it would pick an arbitrary winner via iteration
    // order (#1257). Such keys are tracked so Pass 2 leaves their ghosts intact.
    let mut loc_nodes: IndexMap<(String, String), String> = IndexMap::new();
    let mut loc_collisions: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut loc_ast_keys: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for (nid, attrs) in graph.nodes() {
        let Some(key) = ghost_key(attrs) else {
            continue;
        };
        let is_ast = attrs.get("_origin").and_then(Value::as_str) == Some("ast");
        let has_loc = attrs.get("source_location").is_some_and(is_truthy);
        if !is_ast && !has_loc {
            continue;
        }
        if is_ast {
            // A second AST node on a key already held by an AST node is an
            // ambiguous collision; AST-origin nodes always overwrite a prior entry.
            if loc_ast_keys.contains(&key) {
                loc_collisions.insert(key.clone());
            }
            loc_ast_keys.insert(key.clone());
            loc_nodes.insert(key, nid.clone());
        } else if !loc_nodes.contains_key(&key) {
            loc_nodes.insert(key, nid.clone());
        }
    }

    // Pass 2: a non-AST node sharing a key with a different canonical node is a
    // ghost (last ghost per key, mirroring the Python dict overwrite).
    let mut noloc_nodes: IndexMap<(String, String), String> = IndexMap::new();
    for (nid, attrs) in graph.nodes() {
        if attrs.get("_origin").and_then(Value::as_str) == Some("ast") {
            continue; // AST nodes are never ghosts
        }
        let Some(key) = ghost_key(attrs) else {
            continue;
        };
        if loc_collisions.contains(&key) {
            continue; // ambiguous key: no safe canonical winner, leave ghost intact
        }
        if loc_nodes.get(&key).is_some_and(|canon| canon != nid) {
            noloc_nodes.insert(key, nid.clone());
        }
    }

    let mut ghost_remap: IndexMap<String, String> = IndexMap::new();
    for (key, ghost_id) in &noloc_nodes {
        if let Some(canonical) = loc_nodes.get(key) {
            ghost_remap.insert(ghost_id.clone(), canonical.clone());
        }
    }

    let ghost_ids: Vec<String> = ghost_remap.keys().cloned().collect();
    graph.remove_nodes_from(ghost_ids.iter().map(String::as_str));
    ghost_remap
}

/// Build the normalised-ID lookup table, injecting `ghost_id -> canonical_id`
/// remaps so edges referencing a removed ghost node re-point to its canonical
/// AST replacement. Resolution always normalises the lookup key, so only the
/// normalised ghost-id mapping is consulted.
fn build_norm_to_id(
    node_ids: &IndexSet<String>,
    ghost_remap: &IndexMap<String, String>,
) -> IndexMap<String, String> {
    let mut norm_to_id: IndexMap<String, String> = node_ids
        .iter()
        .map(|nid| (normalize_id(nid), nid.clone()))
        .collect();
    for (ghost_id, canonical_id) in ghost_remap {
        norm_to_id.insert(normalize_id(ghost_id), canonical_id.clone());
    }
    norm_to_id
}

/// Snapshot each node's `source_file` (id → path) so the cross-language `calls`
/// INFERRED filter and the legacy-id alias index can resolve without
/// re-borrowing `graph` inside the per-edge closure.
fn snapshot_source_files(graph: &Graph) -> IndexMap<String, String> {
    graph
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
        .collect()
}

/// Resolve an edge's `source_file`: keep an explicit truthy value, otherwise
/// backfill from the source then target node (#1279). The result is relativised
/// against `root_str`. Returns `None` only when the edge carries a non-string
/// `source_file`, which is left untouched.
fn edge_source_file(
    current: Option<&Value>,
    src_file: &str,
    tgt_file: &str,
    root_str: Option<&str>,
) -> Option<String> {
    let raw = match current {
        Some(v) if is_truthy(v) => v.as_str()?.to_string(),
        _ if src_file.is_empty() => tgt_file.to_string(),
        _ => src_file.to_string(),
    };
    Some(norm_source_file(&raw, root_str))
}

pub(crate) fn add_edges(
    graph: &mut Graph,
    extraction: &Value,
    root_str: Option<&str>,
    ghost_remap: &IndexMap<String, String>,
) {
    let Some(edges) = extraction
        .as_object()
        .and_then(|o| o.get("edges"))
        .and_then(Value::as_array)
    else {
        return;
    };
    let node_ids: IndexSet<String> = graph.nodes().map(|(id, _)| id.clone()).collect();
    let mut norm_to_id = build_norm_to_id(&node_ids, ghost_remap);
    let node_source_files = snapshot_source_files(graph);
    // Pre-migration alias index (#1504): register each canonical node's OLD-stem
    // id forms so a stale-id edge endpoint from an un-re-keyed fragment still
    // resolves to the migrated node instead of dangling.
    crate::migrate::register_legacy_id_aliases(&mut norm_to_id, &node_source_files);

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
            attrs.insert(k.clone(), v.clone());
        }
        // Resolve source_file: keep an explicit value, else backfill from the
        // endpoint nodes (#1279), then relativise against the root.
        let src_file = node_source_files
            .get(&resolved_src)
            .map_or("", String::as_str);
        let tgt_file = node_source_files
            .get(&resolved_tgt)
            .map_or("", String::as_str);
        let resolved_sf = edge_source_file(attrs.get("source_file"), src_file, tgt_file, root_str);
        if let Some(sf) = resolved_sf {
            attrs.insert("source_file".to_string(), Value::String(sf));
        }
        attrs.insert("_src".to_string(), Value::String(resolved_src.clone()));
        attrs.insert("_tgt".to_string(), Value::String(resolved_tgt.clone()));
        Some((resolved_src, resolved_tgt, attrs))
    };

    // Iterate edges in a deterministic order. The graph is undirected and
    // stores direction in `_src`/`_tgt`; when two edges collapse onto the same
    // node pair the last write wins, so an unstable order would flip those
    // fields run-to-run and churn the serialized graph. Sorting on
    // (source, target, relation) — matching graphify-py — fixes the outcome.
    let mut sorted_edges: Vec<&Value> = edges.iter().collect();
    sorted_edges.sort_by_cached_key(|e| edge_sort_key(e));

    let resolved: Vec<(String, String, IndexMap<String, Value>)> =
        if sorted_edges.len() >= PARALLEL_EDGE_THRESHOLD {
            sorted_edges
                .par_iter()
                .filter_map(|e| resolve_edge(e))
                .collect()
        } else {
            sorted_edges
                .iter()
                .filter_map(|e| resolve_edge(e))
                .collect()
        };

    // Bulk insert — O(N + E) instead of the O(N²) shape that
    // per-call `add_edge` would produce when scanning for duplicates.
    graph.bulk_add_edges(resolved);
}
