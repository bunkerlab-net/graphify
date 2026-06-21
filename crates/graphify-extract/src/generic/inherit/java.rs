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
                if sub.kind() == "type_identifier" {
                    emit(
                        &read_text_owned(sub, source),
                        "inherits",
                        nodes,
                        edges,
                        seen_ids,
                    );
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
                            if tid.kind() == "type_identifier" {
                                emit(
                                    &read_text_owned(tid, source),
                                    "implements",
                                    nodes,
                                    edges,
                                    seen_ids,
                                );
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
                                        if tid.kind() == "type_identifier" {
                                            emit(
                                                &read_text_owned(tid, source),
                                                "inherits",
                                                nodes,
                                                edges,
                                                seen_ids,
                                            );
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
