//! PHP inheritance-edge emitter.

#![allow(clippy::cast_possible_truncation)]

use super::emit_base_node;
use crate::generic::walk::{add_edge, first_child_kind, named_children};
use crate::types::{Edge, Node as GNode};
use std::collections::HashSet;
use tree_sitter::Node;

/// Emit `inherits` (`extends`) / `implements` (`implements`) / `mixes_in`
/// (trait `use`) edges for a PHP class. Mirrors Python `_extract_php`.
pub(crate) fn emit_php_inheritance(
    ctx: &mut crate::generic::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    _line: u32,
) {
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;

    let emit = |base_name: Option<String>,
                rel: &str,
                at_line: u32,
                nodes: &mut Vec<GNode>,
                edges: &mut Vec<Edge>,
                seen_ids: &mut HashSet<String>| {
        let Some(base_name) = base_name else { return };
        if base_name.is_empty() {
            return;
        }
        let base_nid = emit_base_node(&base_name, at_line, stem, str_path, nodes, seen_ids);
        add_edge(class_nid, &base_nid, rel, at_line, str_path, None, edges);
    };

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            let child_line = child.start_position().row as u32 + 1;
            match child.kind() {
                "base_clause" => {
                    for sub in named_children(child) {
                        if matches!(sub.kind(), "name" | "qualified_name") {
                            emit(
                                crate::generic::references::php_name_text(sub, source),
                                "inherits",
                                child_line,
                                nodes,
                                edges,
                                seen_ids,
                            );
                        }
                    }
                }
                "class_interface_clause" => {
                    for sub in named_children(child) {
                        if matches!(sub.kind(), "name" | "qualified_name") {
                            emit(
                                crate::generic::references::php_name_text(sub, source),
                                "implements",
                                child_line,
                                nodes,
                                edges,
                                seen_ids,
                            );
                        }
                    }
                }
                _ => {}
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    // Trait `use` declarations inside the class body → `mixes_in`.
    let body = node
        .child_by_field_name("body")
        .or_else(|| first_child_kind(node, "declaration_list"));
    if let Some(body) = body {
        for member in named_children(body) {
            if member.kind() != "use_declaration" {
                continue;
            }
            let member_line = member.start_position().row as u32 + 1;
            for sub in named_children(member) {
                if matches!(sub.kind(), "name" | "qualified_name") {
                    emit(
                        crate::generic::references::php_name_text(sub, source),
                        "mixes_in",
                        member_line,
                        nodes,
                        edges,
                        seen_ids,
                    );
                }
            }
        }
    }
}
