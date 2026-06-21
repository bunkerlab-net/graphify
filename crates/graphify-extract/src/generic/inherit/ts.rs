//! TypeScript / JavaScript inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::walk::add_edge;
use tree_sitter::Node;

/// Emit `inherits` / `implements` edges for a TS class declaration's
/// `class_heritage` child.
///
/// TS distinguishes `extends_clause` (single class) from `implements_clause`
/// (one or more interfaces). `extends` is normalised to `inherits` so all
/// languages share a single relation name for class extension. The `name`
/// field's type-arguments are NOT walked here — that happens in the field /
/// method passes via `ts_collect_type_refs`.
pub(crate) fn emit_ts_inheritance(
    ctx: &mut crate::generic::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
) {
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "class_heritage" {
            let mut hcur = child.walk();
            if hcur.goto_first_child() {
                loop {
                    let clause = hcur.node();
                    let relation = match clause.kind() {
                        "extends_clause" => Some("inherits"),
                        "implements_clause" => Some("implements"),
                        _ => None,
                    };
                    if let Some(rel) = relation {
                        for name in
                            crate::generic::references::ts_heritage_clause_entries(clause, source)
                        {
                            let base_nid =
                                emit_base_node(&name, line, stem, str_path, nodes, seen_ids);
                            add_edge(class_nid, &base_nid, rel, line, str_path, None, edges);
                        }
                    }
                    if !hcur.goto_next_sibling() {
                        break;
                    }
                }
            }
        } else if child.kind() == "extends_type_clause" {
            // Interface heritage (`interface A extends B, C`) is an
            // extends_type_clause node directly under the declaration, NOT
            // wrapped in class_heritage. Its base entries are the same node types
            // extends_clause holds, so the entry helper is reusable. Without this
            // branch interface inheritance is dropped entirely (#1095).
            for name in crate::generic::references::ts_heritage_clause_entries(child, source) {
                let base_nid = emit_base_node(&name, line, stem, str_path, nodes, seen_ids);
                add_edge(
                    class_nid, &base_nid, "inherits", line, str_path, None, edges,
                );
            }
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}
