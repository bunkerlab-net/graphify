//! Cross-file Swift member-call resolution.

use super::java::is_type_like_definition;
use crate::types::{Edge, Node, RawCall};
use std::collections::HashMap;
use std::path::PathBuf;

/// Re-parse a Swift file's AST into a `local name -> type name` table, from
/// property declarations (type annotation, else constructor inference) and
/// function parameters. Feeds [`resolve_swift_member_calls`]. Rebuilt by
/// re-parsing (like the Java type-reference pass) rather than threaded through a
/// `FileResult` sidecar.
fn collect_swift_type_table(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    table: &mut HashMap<String, String>,
) {
    use crate::generic::references::{
        RefRole, swift_collect_type_refs, swift_constructor_type, swift_property_name,
        swift_property_type_node,
    };
    match node.kind() {
        "property_declaration" => {
            let mut prop_type: Option<String> = None;
            if let Some(anno) = swift_property_type_node(node) {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                swift_collect_type_refs(anno, source, false, &mut refs);
                prop_type = refs
                    .into_iter()
                    .find(|(_, r)| *r == RefRole::Direct)
                    .map(|(n, _)| n);
            }
            if prop_type.is_none() {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "call_expression"
                            && let Some(ctor) = swift_constructor_type(cur.node(), source)
                        {
                            prop_type = Some(ctor);
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if let (Some(name), Some(ty)) = (swift_property_name(node, source), prop_type) {
                table.insert(name, ty);
            }
        }
        "parameter" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                swift_collect_type_refs(type_node, source, false, &mut refs);
                if let Some((ty, _)) = refs.into_iter().find(|(_, r)| *r == RefRole::Direct)
                    && let Some(name_node) = node.child_by_field_name("name")
                {
                    let pname = name_node.utf8_text(source).unwrap_or("");
                    if !pname.is_empty() {
                        table.insert(pname.to_string(), ty);
                    }
                }
            }
        }
        _ => {}
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            collect_swift_type_table(cur.node(), source, table);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Resolve cross-file Swift member calls (`recv.method()`) to the receiver's
/// real type definition (#1356). The shared call pass drops every
/// `is_member_call` (a bare method name collides across the corpus); this pass
/// types the receiver via the file's local type table (or treats an upper-cased
/// receiver as a type itself), then emits an edge ONLY when the type name
/// resolves to exactly one definition (god-node guard). Everything it adds is
/// INFERRED (type inference, not an explicit import).
#[allow(clippy::too_many_lines)] // linear: re-parse type tables, build indexes, resolve each member call
pub(super) fn resolve_swift_member_calls(
    swift_paths: &[PathBuf],
    all_nodes: &[Node],
    all_edges: &mut Vec<Edge>,
    all_raw_calls: &[RawCall],
) {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .is_err()
    {
        return;
    }
    let mut type_table_by_file: HashMap<String, HashMap<String, String>> = HashMap::new();
    for path in swift_paths {
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut table: HashMap<String, String> = HashMap::new();
        collect_swift_type_table(tree.root_node(), &source, &mut table);
        type_table_by_file.insert(path.to_string_lossy().into_owned(), table);
    }
    if type_table_by_file.is_empty() {
        return;
    }

    let key = |s: &str| -> String {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase()
    };

    // A genuine type is the target of a `contains` edge from its file; bare type
    // references create same-label shadow nodes that are NOT contained, so this
    // keeps a shadow from making a real type name look ambiguous.
    let contained: std::collections::HashSet<&str> = all_edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| e.target.as_str())
        .collect();
    let mut type_def_nids: HashMap<String, Vec<String>> = HashMap::new();
    let mut node_by_id: HashMap<&str, &Node> = HashMap::new();
    for n in all_nodes {
        node_by_id.insert(n.id.as_str(), n);
        if !n.source_file.is_empty()
            && contained.contains(n.id.as_str())
            && is_type_like_definition(n)
        {
            type_def_nids
                .entry(key(n.label.as_str()))
                .or_default()
                .push(n.id.clone());
        }
    }

    // (type_node_id, method_key) -> method_node_id, from `method` edges.
    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    for e in all_edges.iter() {
        if e.relation == "method"
            && let Some(tnode) = node_by_id.get(e.target.as_str())
        {
            method_index.insert(
                (e.source.clone(), key(tnode.label.as_str())),
                e.target.clone(),
            );
        }
    }

    let mut existing_pairs: std::collections::HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if !rc.is_member_call || rc.callee.is_empty() || rc.caller_nid.is_empty() {
            continue;
        }
        let Some(receiver) = rc.receiver.as_deref() else {
            continue;
        };
        // An upper-cased receiver is itself a type (`Type.staticMethod()`,
        // `Singleton.shared.x()`); otherwise look it up in the declaring file's
        // local type table.
        let type_name = if receiver.chars().next().is_some_and(char::is_uppercase) {
            receiver.to_string()
        } else if let Some(t) = type_table_by_file
            .get(&rc.source_file)
            .and_then(|tbl| tbl.get(receiver))
        {
            t.clone()
        } else {
            continue;
        };
        let type_nid = match type_def_nids.get(&key(type_name.as_str())) {
            Some(defs) if defs.len() == 1 => &defs[0],
            _ => continue, // ambiguous or absent -> god-node guard
        };
        let (target, relation) =
            match method_index.get(&(type_nid.clone(), key(rc.callee.as_str()))) {
                Some(method) => (method.clone(), "calls"),
                None => (type_nid.clone(), "references"),
            };
        if target == rc.caller_nid
            || existing_pairs.contains(&(rc.caller_nid.clone(), target.clone()))
        {
            continue;
        }
        existing_pairs.insert((rc.caller_nid.clone(), target.clone()));
        new_edges.push(Edge {
            external: false,
            source: rc.caller_nid.clone(),
            target,
            relation: relation.to_string(),
            confidence: "INFERRED".to_string(),
            source_file: rc.source_file.clone(),
            source_location: Some(rc.source_location.clone()),
            weight: 1.0,
            context: Some("call".to_string()),
            confidence_score: Some(0.8),
        });
    }
    all_edges.extend(new_edges);
}

// ── Main extract() ────────────────────────────────────────────────────────────
