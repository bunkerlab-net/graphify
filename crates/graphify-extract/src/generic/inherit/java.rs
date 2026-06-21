//! Java inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::names::read_text_owned;
use crate::generic::walk::add_edge;
use crate::types::{Edge, Node as GNode};
use std::collections::HashSet;
use tree_sitter::Node;

/// Emit `inherits` and `implements` edges for a Java class or interface node.
///
/// Java's source-level `extends` keyword (class extending a superclass or
/// interface extending other interfaces) is normalised to the `inherits`
/// relation so cross-language consumers see the same shape as C#, Swift, and
/// C++. `implements` (class implementing an interface) is kept as-is. All
/// three cases are handled here to match Python `_extract_java`.
#[allow(clippy::too_many_lines)] // sequential dispatch over Java's three inheritance shapes
pub(crate) fn emit_java_inheritance(
    ctx: &mut crate::generic::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    node_type: &str,
    line: u32,
) {
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let emit = |base_name: &str,
                rel: &str,
                nodes: &mut Vec<GNode>,
                edges: &mut Vec<Edge>,
                seen_ids: &mut HashSet<String>| {
        if base_name.is_empty() {
            return;
        }
        let base_nid = emit_base_node(base_name, line, stem, str_path, nodes, seen_ids);
        add_edge(class_nid, &base_nid, rel, line, str_path, None, edges);
    };

    if let Some(sup) = node.child_by_field_name("superclass") {
        let mut cur = sup.walk();
        if cur.goto_first_child() {
            loop {
                let sub = cur.node();
                if let Some(name) = java_base_name(sub, source) {
                    emit(&name, "inherits", nodes, edges, seen_ids);
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    if let Some(ifs) = node.child_by_field_name("interfaces") {
        let mut cur = ifs.walk();
        if cur.goto_first_child() {
            loop {
                let sub = cur.node();
                if sub.kind() == "type_list" {
                    let mut tcur = sub.walk();
                    if tcur.goto_first_child() {
                        loop {
                            let tid = tcur.node();
                            if let Some(name) = java_base_name(tid, source) {
                                emit(&name, "implements", nodes, edges, seen_ids);
                            }
                            if !tcur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    if node_type == "interface_declaration" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "extends_interfaces" {
                    let mut scur = child.walk();
                    if scur.goto_first_child() {
                        loop {
                            let sub = scur.node();
                            if sub.kind() == "type_list" {
                                let mut tcur = sub.walk();
                                if tcur.goto_first_child() {
                                    loop {
                                        let tid = tcur.node();
                                        if let Some(name) = java_base_name(tid, source) {
                                            emit(&name, "inherits", nodes, edges, seen_ids);
                                        }
                                        if !tcur.goto_next_sibling() {
                                            break;
                                        }
                                    }
                                }
                            }
                            if !scur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Extract the base type name from a Java inheritance entry: a plain
/// `type_identifier`, a qualified `scoped_type_identifier` (tail after the
/// final `.`), or a `generic_type` (its base, qualified-tail when scoped).
/// Returns `None` for non-type nodes such as the `extends` keyword.
///
/// Divergence from graphify-py `_extract_java` (extract.py:2777-2799), which
/// matches only `type_identifier` and silently drops qualified/generic bases.
fn java_base_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => {
            let name = read_text_owned(node, source);
            (!name.is_empty()).then_some(name)
        }
        "scoped_type_identifier" => {
            let text = read_text_owned(node, source);
            let tail = text.rsplit('.').next().unwrap_or(&text);
            (!tail.is_empty()).then(|| tail.to_string())
        }
        "generic_type" => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if matches!(child.kind(), "type_identifier" | "scoped_type_identifier") {
                        return java_base_name(child, source);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        }
        _ => None,
    }
}
