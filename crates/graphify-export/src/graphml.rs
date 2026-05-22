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

use std::fmt::Write as FmtWrite;
use std::path::Path;

use graphify_build::Graph;
use indexmap::{IndexMap, IndexSet};
use serde_json::Value;

use crate::{ExportError, node_community_map};

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
#[allow(clippy::too_many_lines)] // Inherent complexity of a full GraphML serialiser
pub fn to_graphml(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_path: &Path,
) -> Result<(), ExportError> {
    let node_community = node_community_map(communities);

    // Discover all node attribute keys (excluding `id` which is the GraphML id)
    let mut node_keys: IndexSet<String> = IndexSet::new();
    node_keys.insert("community".to_string());
    for (_, attrs) in graph.nodes() {
        for k in attrs.keys() {
            if k != "id" {
                node_keys.insert(k.clone());
            }
        }
    }

    // Discover all edge attribute keys
    let mut edge_keys: IndexSet<String> = IndexSet::new();
    for edge in graph.edges() {
        for k in edge.attrs.keys() {
            if k != "_src" && k != "_tgt" {
                edge_keys.insert(k.clone());
            }
        }
    }

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<graphml xmlns=\"http://graphml.graphdrawing.org/graphml\"\n");
    out.push_str("         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n");
    out.push_str("         xsi:schemaLocation=\"http://graphml.graphdrawing.org/graphml\n");
    out.push_str("           http://graphml.graphdrawing.org/graphml/1.0/graphml.xsd\">\n");

    // Node key declarations
    for key in &node_keys {
        // Infallible write to String
        let _ = writeln!(
            out,
            "  <key id=\"d_{key}\" for=\"node\" attr.name=\"{key}\" attr.type=\"string\"/>"
        );
    }
    // Edge key declarations
    for key in &edge_keys {
        let _ = writeln!(
            out,
            "  <key id=\"e_{key}\" for=\"edge\" attr.name=\"{key}\" attr.type=\"string\"/>"
        );
    }

    let directed_str = if graph.kind.is_directed() {
        "directed"
    } else {
        "undirected"
    };
    let _ = writeln!(out, "  <graph id=\"G\" edgedefault=\"{directed_str}\">");

    // Nodes
    for (node_id, attrs) in graph.nodes() {
        let id_esc = xml_escape(node_id);
        let _ = writeln!(out, "    <node id=\"{id_esc}\">");
        let cid = node_community.get(node_id).copied().unwrap_or(-1);
        let _ = writeln!(out, "      <data key=\"d_community\">{cid}</data>");
        for key in &node_keys {
            if key == "community" {
                continue;
            }
            if let Some(val) = attrs.get(key) {
                let s = value_to_str(val);
                let s_esc = xml_escape(&s);
                let _ = writeln!(out, "      <data key=\"d_{key}\">{s_esc}</data>");
            }
        }
        out.push_str("    </node>\n");
    }

    // Edges
    for (idx, edge) in graph.edges().enumerate() {
        let src_esc = xml_escape(&edge.source);
        let tgt_esc = xml_escape(&edge.target);
        let _ = writeln!(
            out,
            "    <edge id=\"e{idx}\" source=\"{src_esc}\" target=\"{tgt_esc}\">"
        );
        for key in &edge_keys {
            if let Some(val) = edge.attrs.get(key) {
                let s = value_to_str(val);
                let s_esc = xml_escape(&s);
                let _ = writeln!(out, "      <data key=\"e_{key}\">{s_esc}</data>");
            }
        }
        out.push_str("    </edge>\n");
    }

    out.push_str("  </graph>\n");
    out.push_str("</graphml>\n");

    std::fs::write(output_path, out)?;
    Ok(())
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
