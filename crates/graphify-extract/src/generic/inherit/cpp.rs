//! C++ inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::names::read_text_owned;
use crate::generic::walk::add_edge;
use tree_sitter::Node;

/// Emit `inherits` edges from a C++ `base_class_clause` node.
///
/// C++ allows multiple inheritance; all entries in the clause produce
/// `inherits` edges regardless of access specifier (`public`, `protected`,
/// `private`), matching Python `_extract_cpp`.
pub(crate) fn emit_cpp_inheritance(
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
        if child.kind() == "base_class_clause" {
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    let base = match sub.kind() {
                        "type_identifier" => Some(read_text_owned(sub, source)),
                        "qualified_identifier" | "template_type" => {
                            if let Some(tail) = sub.child_by_field_name("name") {
                                Some(read_text_owned(tail, source))
                            } else {
                                Some(read_text_owned(sub, source))
                            }
                        }
                        _ => None,
                    };
                    if let Some(b) = base
                        && !b.is_empty()
                    {
                        let base_nid = emit_base_node(&b, line, stem, str_path, nodes, seen_ids);
                        add_edge(
                            class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
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
