//! `GraphML` export — `to_graphml`.
//!
//! Mirrors Python `to_graphml` from `graphify-py/graphify/export.py`.
//!
//! Python uses `networkx.write_graphml()`. We produce equivalent XML by hand:
//! a standard `GraphML` envelope with one `<key>` declaration per attribute,
//! then nodes and edges with `<data>` children.
//!
//! The test assertions are:
//! - `<graphml` appears in content
//! - `<node` appears in content
//! - `community` appears in content

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};

use graphify_build::Graph;
use indexmap::IndexMap;
use serde_json::Value;

use crate::{ExportError, node_community_map};

/// Per-process sequence for unique `GraphML` temp-file names (atomic write).
static GRAPHML_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Export graph as a `GraphML` file.
///
/// Community IDs are written as a node attribute so tools like Gephi can colour
/// by community. Edge confidence is preserved as an edge attribute.
///
/// Mirrors Python `to_graphml`.
///
/// # Errors
///
/// Returns [`ExportError::Io`] on file-write failure.
pub fn to_graphml(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_path: &Path,
) -> Result<(), ExportError> {
    let node_community = node_community_map(communities);
    let keys = KeyRegistry::build(graph);

    let mut out = String::new();
    write_graphml_preamble(&mut out, &keys, graph.kind.is_directed());
    write_graphml_graph_data(&mut out, graph, &keys);
    write_graphml_nodes(&mut out, graph, &node_community, &keys);
    write_graphml_edges(&mut out, graph, &keys);
    out.push_str("  </graph>\n");
    out.push_str("</graphml>\n");

    // Write atomically (#1831): a mid-write failure otherwise leaves a partial
    // (0-byte) `.graphml` that downstream tooling mistakes for a completed
    // export, or truncates an existing good file. Stage a sibling temp, then
    // rename over the destination on success; clean up the temp either way.
    // The temp name carries a process id + per-call sequence so two exports to
    // the same path never clobber each other's temp (graphify-py's fixed
    // `<out>.tmp` is not collision-safe; a public writer should be).
    let seq = GRAPHML_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp_os = output_path.as_os_str().to_os_string();
    tmp_os.push(format!(".{}.{seq}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp_os);
    let result =
        std::fs::write(&tmp, out.as_bytes()).and_then(|()| std::fs::rename(&tmp, output_path));
    if tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

/// The `GraphML` `attr.type` of a single value. Scalars keep their native type
/// so a consumer reads them back native; every non-scalar (string, null,
/// array, object) uses `string` — arrays/objects are then JSON-serialised by
/// [`value_to_str`], matching Python's `json.dumps` (#1831).
fn value_graphml_type(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "double",
        Value::Number(_) => "long",
        _ => "string",
    }
}

/// Whether a node attribute key is exported. Mirrors graphify-py's `to_graphml`,
/// which drops every `_`-prefixed key (AST-provenance `_origin`, the `_src`/
/// `_tgt` direction markers) plus the node's own `id`/`community` (the latter is
/// synthesised from the community map, not read from the attr map).
fn is_exported_node_key(key: &str) -> bool {
    !key.starts_with('_') && key != "id" && key != "community"
}

/// Whether an edge attribute key is exported. graphify-py only drops `_`-prefixed
/// edge attrs, so arbitrary edge data named `id`/`community` is preserved.
fn is_exported_edge_key(key: &str) -> bool {
    !key.starts_with('_')
}

/// A registry of `GraphML` `<key>` declarations across all scopes.
///
/// `networkx.write_graphml` assigns each distinct `(scope, attr.name,
/// attr.type)` a sequential opaque id (`d0`, `d1`, …) and splits a genuinely
/// mixed-type attribute into one `<key>` per type, so every value is written
/// under a key whose declared type matches it. We do the same: opaque ids side-
/// step both id collisions and attribute names that are invalid XML ids (the
/// real name lives only in the escaped `attr.name`).
struct KeyRegistry {
    /// `(id, scope, name, type)` in first-seen order, for `<key>` declarations.
    decls: Vec<(String, &'static str, String, &'static str)>,
    /// `(scope, name, type)` -> id, for per-value `<data>` emission.
    ids: HashMap<(&'static str, String, &'static str), String>,
}

impl KeyRegistry {
    /// Walk every exported attribute (node community + node/edge/graph attrs) in
    /// document order, registering one key per distinct `(scope, name, type)`.
    fn build(graph: &Graph) -> Self {
        let mut reg = Self {
            decls: Vec::new(),
            ids: HashMap::new(),
        };
        for (_, attrs) in graph.nodes() {
            reg.register("node", "community", "long");
            for (k, v) in attrs {
                if is_exported_node_key(k) {
                    reg.register("node", k, value_graphml_type(v));
                }
            }
        }
        for edge in graph.edges() {
            for (k, v) in &edge.attrs {
                if is_exported_edge_key(k) {
                    reg.register("edge", k, value_graphml_type(v));
                }
            }
        }
        for (k, v) in &graph.graph_attrs {
            // Drop internal `_`-prefixed markers from graph scope too, consistent
            // with node/edge keys — they are runtime/persistence details, not
            // graph data. `hyperedges` (the only real graph attr) is unaffected.
            if !k.starts_with('_') {
                reg.register("graph", k, value_graphml_type(v));
            }
        }
        reg
    }

    fn register(&mut self, scope: &'static str, name: &str, ty: &'static str) {
        let key = (scope, name.to_string(), ty);
        if self.ids.contains_key(&key) {
            return;
        }
        let id = format!("d{}", self.decls.len());
        self.decls.push((id.clone(), scope, name.to_string(), ty));
        self.ids.insert(key, id);
    }

    /// The key id for a value in the given scope (always registered by `build`).
    fn id_for(&self, scope: &'static str, name: &str, v: &Value) -> Option<&str> {
        self.ids
            .get(&(scope, name.to_string(), value_graphml_type(v)))
            .map(String::as_str)
    }
}

/// Write the XML header, namespace declaration, key declarations, and `<graph>` element.
fn write_graphml_preamble(out: &mut String, keys: &KeyRegistry, directed: bool) {
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<graphml xmlns=\"http://graphml.graphdrawing.org/graphml\"\n");
    out.push_str("         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n");
    out.push_str("         xsi:schemaLocation=\"http://graphml.graphdrawing.org/graphml\n");
    out.push_str("           http://graphml.graphdrawing.org/graphml/1.0/graphml.xsd\">\n");
    for (id, scope, name, ty) in &keys.decls {
        let name_esc = xml_escape(name);
        let _ = writeln!(
            out,
            "  <key id=\"{id}\" for=\"{scope}\" attr.name=\"{name_esc}\" attr.type=\"{ty}\"/>"
        );
    }
    let directed_str = if directed { "directed" } else { "undirected" };
    let _ = writeln!(out, "  <graph id=\"G\" edgedefault=\"{directed_str}\">");
}

/// Write a single `<data>` child for `val` under the scope's matching key.
fn write_data(out: &mut String, keys: &KeyRegistry, scope: &'static str, name: &str, val: &Value) {
    if let Some(id) = keys.id_for(scope, name, val) {
        let s_esc = xml_escape(&value_to_str(val));
        let _ = writeln!(out, "      <data key=\"{id}\">{s_esc}</data>");
    }
}

/// Write graph-level `<data>` children (must precede nodes per the `GraphML` DTD).
fn write_graphml_graph_data(out: &mut String, graph: &Graph, keys: &KeyRegistry) {
    for (name, val) in &graph.graph_attrs {
        if let Some(id) = keys.id_for("graph", name, val) {
            let s_esc = xml_escape(&value_to_str(val));
            let _ = writeln!(out, "    <data key=\"{id}\">{s_esc}</data>");
        }
    }
}

/// Write `<node>` blocks with the community + per-key `<data>` children.
fn write_graphml_nodes(
    out: &mut String,
    graph: &Graph,
    node_community: &IndexMap<String, i64>,
    keys: &KeyRegistry,
) {
    for (node_id, attrs) in graph.nodes() {
        let id_esc = xml_escape(node_id);
        let _ = writeln!(out, "    <node id=\"{id_esc}\">");
        let cid = node_community.get(node_id).copied().unwrap_or(-1);
        write_data(out, keys, "node", "community", &Value::from(cid));
        for (name, val) in attrs {
            if is_exported_node_key(name) {
                write_data(out, keys, "node", name, val);
            }
        }
        out.push_str("    </node>\n");
    }
}

/// Write `<edge>` blocks with per-key `<data>` children.
fn write_graphml_edges(out: &mut String, graph: &Graph, keys: &KeyRegistry) {
    for (idx, edge) in graph.edges().enumerate() {
        let src_esc = xml_escape(&edge.source);
        let tgt_esc = xml_escape(&edge.target);
        let _ = writeln!(
            out,
            "    <edge id=\"e{idx}\" source=\"{src_esc}\" target=\"{tgt_esc}\">"
        );
        for (name, val) in &edge.attrs {
            if is_exported_edge_key(name) {
                write_data(out, keys, "edge", name, val);
            }
        }
        out.push_str("    </edge>\n");
    }
}

/// Convert a `serde_json::Value` to a string for XML embedding.
fn value_to_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// XML-escape a string (minimal: `&`, `<`, `>`, `"`, `'`).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
