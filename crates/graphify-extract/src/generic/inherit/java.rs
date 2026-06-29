//! Java inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::references::{RefRole, java_collect_type_refs};
use crate::generic::walk::add_edge;
use crate::types::{Edge, Node as GNode};
use std::collections::HashSet;
use tree_sitter::Node;

/// Emit `inherits` and `implements` edges for a Java class or interface node.
///
/// Java's source-level `extends` keyword (class extending a superclass or
/// interface extending other interfaces) is normalised to the `inherits`
/// relation so cross-language consumers see the same shape as C#, Swift, and
/// C++. `implements` (class implementing an interface) is kept as-is. Type
/// arguments on a generic parent (`extends Bar<Baz>` / `implements List<T>`)
/// emit `references` edges with context `generic_arg` (#1510). Mirrors Python
/// `_extract_java` / `_emit_java_parent_type`.
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
    // Emit the parent edge for the base type, plus a `generic_arg` reference for
    // every type argument inside a generic parent (`extends Bar<Baz>` → Baz).
    let emit_parent_type = |type_node: Node<'_>,
                            rel: &str,
                            nodes: &mut Vec<GNode>,
                            edges: &mut Vec<Edge>,
                            seen_ids: &mut HashSet<String>| {
        let mut refs: Vec<(String, RefRole)> = Vec::new();
        java_collect_type_refs(type_node, source, false, &mut refs);
        let mut parent_emitted = false;
        for (ref_name, role) in refs {
            if ref_name.is_empty() {
                continue;
            }
            match role {
                RefRole::Direct if !parent_emitted => {
                    let base_nid = emit_base_node(&ref_name, line, stem, str_path, nodes, seen_ids);
                    add_edge(class_nid, &base_nid, rel, line, str_path, None, edges);
                    parent_emitted = true;
                }
                RefRole::Generic => {
                    let target = emit_base_node(&ref_name, line, stem, str_path, nodes, seen_ids);
                    if target != class_nid {
                        add_edge(
                            class_nid,
                            &target,
                            "references",
                            line,
                            str_path,
                            Some("generic_arg"),
                            edges,
                        );
                    }
                }
                RefRole::Direct => {}
            }
        }
    };

    // `class Foo extends Bar` -> inherits (first named child of `superclass`).
    if let Some(sup) = node.child_by_field_name("superclass") {
        let mut cur = sup.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    emit_parent_type(cur.node(), "inherits", nodes, edges, seen_ids);
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    // `class Foo implements A, B` -> implements (each type in the `type_list`).
    if let Some(ifs) = node.child_by_field_name("interfaces") {
        let mut cur = ifs.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "type_list" {
                    let mut tcur = cur.node().walk();
                    if tcur.goto_first_child() {
                        loop {
                            if tcur.node().is_named() {
                                emit_parent_type(tcur.node(), "implements", nodes, edges, seen_ids);
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

    // `interface Foo extends A, B` -> inherits.
    if node_type == "interface_declaration" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "extends_interfaces" {
                    let mut scur = cur.node().walk();
                    if scur.goto_first_child() {
                        loop {
                            if scur.node().kind() == "type_list" {
                                let mut tcur = scur.node().walk();
                                if tcur.goto_first_child() {
                                    loop {
                                        if tcur.node().is_named() {
                                            emit_parent_type(
                                                tcur.node(),
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
