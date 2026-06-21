//! PHP type-reference helpers.

use tree_sitter::Node;

use super::{RefRole, recurse_named_refs, role_of};
use crate::generic::names::read_text_owned;

/// Return the unqualified tail of a PHP `name` / `qualified_name` node.
#[must_use]
pub(crate) fn php_name_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let full = read_text_owned(node, source);
    let tail = full.rsplit('\\').next().unwrap_or(&full);
    if tail.is_empty() {
        None
    } else {
        Some(tail.to_string())
    }
}

/// PHP type-node kinds that count as a type annotation on params/properties.
pub(crate) const PHP_TYPE_NODE_KINDS: &[&str] = &[
    "named_type",
    "primitive_type",
    "nullable_type",
    "union_type",
    "intersection_type",
    "optional_type",
];

/// Return the return-type node following `formal_parameters` on a PHP method.
#[must_use]
pub(crate) fn php_method_return_type_node(method_node: Node<'_>) -> Option<Node<'_>> {
    let mut saw_params = false;
    let mut cur = method_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "formal_parameters" {
                saw_params = true;
            } else if saw_params
                && c.is_named()
                && c.kind() != "compound_statement"
                && PHP_TYPE_NODE_KINDS.contains(&c.kind())
            {
                return Some(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Walk a PHP type expression; append `(name, role)` tuples. Mirrors
/// Python `_php_collect_type_refs`.
pub(crate) fn php_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "primitive_type" => {}
        "named_type" => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if matches!(cur.node().kind(), "name" | "qualified_name") {
                        if let Some(text) = php_name_text(cur.node(), source) {
                            out.push((text, role_of(generic)));
                        }
                        return;
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "name" | "qualified_name" => {
            if let Some(text) = php_name_text(node, source) {
                out.push((text, role_of(generic)));
            }
        }
        // `nullable_type` / `union_type` / `intersection_type` / `optional_type`
        // are named wrappers handled identically by the fallback below.
        _ if node.is_named() => {
            recurse_named_refs(node, source, generic, out, php_collect_type_refs);
        }
        _ => {}
    }
}
