//! Scala inheritance-edge emitter.

#![allow(clippy::cast_possible_truncation)]

use crate::generic::names::read_text_owned;
use crate::generic::walk::{add_edge, first_child_kind, named_children};
use tree_sitter::Node;

/// Emit `inherits` (first base after `extends`) / `mixes_in` (each `with`
/// trait) edges plus `references[field]` edges for constructor parameters.
/// Mirrors Python `_extract_scala`.
pub(crate) fn emit_scala_inheritance(
    ctx: &mut crate::generic::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    _line: u32,
) {
    use crate::generic::references::{RefRole, scala_collect_type_refs};
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;

    let extend = node
        .child_by_field_name("extend")
        .or_else(|| first_child_kind(node, "extends_clause"));
    if let Some(extend) = extend {
        let mut bases: Vec<(String, u32)> = Vec::new();
        for c in named_children(extend) {
            let c_line = c.start_position().row as u32 + 1;
            // Skip empty base names (consistent with the PHP emitter) so a
            // malformed node never spawns an empty-label node.
            if c.kind() == "type_identifier" {
                let name = read_text_owned(c, source);
                if !name.is_empty() {
                    bases.push((name, c_line));
                }
            } else if c.kind() == "generic_type" {
                let base = c
                    .child_by_field_name("type")
                    .or_else(|| first_child_kind(c, "type_identifier"));
                if let Some(base) = base {
                    let name = read_text_owned(base, source);
                    if !name.is_empty() {
                        bases.push((name, c_line));
                    }
                }
            }
        }
        for (idx, (base_name, base_line)) in bases.into_iter().enumerate() {
            let rel = if idx == 0 { "inherits" } else { "mixes_in" };
            let base_nid = crate::generic::walk::ensure_named_node(
                &base_name, stem, str_path, nodes, seen_ids,
            );
            if base_nid != class_nid {
                add_edge(class_nid, &base_nid, rel, base_line, str_path, None, edges);
            }
        }
    }

    for c in named_children(node) {
        if c.kind() != "class_parameters" {
            continue;
        }
        for cp in named_children(c) {
            if cp.kind() != "class_parameter" {
                continue;
            }
            let Some(ptype) = cp.child_by_field_name("type") else {
                continue;
            };
            let cp_line = cp.start_position().row as u32 + 1;
            let mut refs: Vec<(String, RefRole)> = Vec::new();
            scala_collect_type_refs(ptype, source, false, &mut refs);
            for (ref_name, role) in refs {
                let context = role.into_context("field");
                let target = crate::generic::walk::ensure_named_node(
                    &ref_name, stem, str_path, nodes, seen_ids,
                );
                if target != class_nid {
                    add_edge(
                        class_nid,
                        &target,
                        "references",
                        cp_line,
                        str_path,
                        Some(context),
                        edges,
                    );
                }
            }
        }
    }
}
