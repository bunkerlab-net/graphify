//! Scala type-reference collector.

use tree_sitter::Node;

use super::{RefRole, recurse_named_refs, role_of};
use crate::generic::names::read_text_owned;
use crate::generic::walk::first_child_kind;

/// Walk a Scala type expression; append `(name, role)` tuples. Mirrors
/// Python `_scala_collect_type_refs`.
pub(crate) fn scala_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        "generic_type" => {
            let base = node
                .child_by_field_name("type")
                .or_else(|| first_child_kind(node, "type_identifier"));
            if let Some(base) = base
                && base.kind() == "type_identifier"
            {
                let text = read_text_owned(base, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        recurse_named_refs(cur.node(), source, true, out, scala_collect_type_refs);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "compound_type" | "infix_type" | "function_type" | "tuple_type" | "annotated_type"
        | "projected_type" => {
            recurse_named_refs(node, source, generic, out, scala_collect_type_refs);
        }
        // No catch-all recurse: graphify-py's `_scala_collect_type_refs`
        // (extract.py) handles only `type_identifier`, `generic_type`, and the
        // wrapper kinds above, so other named nodes are intentionally ignored to
        // preserve parity.
        _ => {}
    }
}
