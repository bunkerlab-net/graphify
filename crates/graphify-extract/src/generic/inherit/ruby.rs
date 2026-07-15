//! Ruby inheritance-edge emitter.

#![allow(clippy::cast_possible_truncation)]

use tree_sitter::Node;

use crate::generic::names::read_text_owned;
use crate::generic::walk::{add_edge, ensure_named_node, named_children};

/// Emit an `inherits` edge for a Ruby `class Dog < Animal` superclass.
///
/// The base class sits in the `superclass` field as a `constant` or a
/// `scope_resolution` (`A::B::Base`, whose LAST constant is the base name).
/// Mirrors the Ruby branch added to graphify-py `_extract_generic` (a19b9e9):
/// without it every Ruby `inherits` edge was silently dropped.
pub(crate) fn emit_ruby_inheritance(
    ctx: &mut crate::generic::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
) {
    let Some(sup) = node.child_by_field_name("superclass") else {
        return;
    };
    let mut base = String::new();
    for sub in named_children(sup) {
        match sub.kind() {
            "constant" => {
                base = read_text_owned(sub, source);
                break;
            }
            "scope_resolution" => {
                // `A::B::Base` → the last `constant` child is the base name.
                let consts: Vec<Node<'_>> = named_children(sub)
                    .into_iter()
                    .filter(|c| c.kind() == "constant")
                    .collect();
                if let Some(c) = consts.last() {
                    base = read_text_owned(*c, source);
                }
                break;
            }
            _ => {}
        }
    }
    if base.is_empty() {
        return;
    }
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let base_nid = ensure_named_node(&base, stem, str_path, ctx.nodes, ctx.seen_ids);
    if base_nid != class_nid {
        add_edge(
            class_nid, &base_nid, "inherits", line, str_path, None, ctx.edges,
        );
    }
}
