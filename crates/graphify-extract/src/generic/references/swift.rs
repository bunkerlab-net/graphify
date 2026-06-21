//! Swift type-reference and property/constructor helpers.

use tree_sitter::Node;

use super::{RefRole, recurse_named_refs, role_of};
use crate::generic::names::read_text_owned;
use crate::generic::walk::first_child_kind;

/// Return the head `type_identifier` text from a Swift `user_type` node.
#[must_use]
pub(crate) fn swift_user_type_name(user_type_node: Node<'_>, source: &[u8]) -> Option<String> {
    first_child_kind(user_type_node, "type_identifier")
        .map(|n| read_text_owned(n, source))
        .filter(|t| !t.is_empty())
}

/// Return the `type_annotation` child of a Swift `property_declaration`, if any.
#[must_use]
pub(crate) fn swift_property_type_node(property_node: Node<'_>) -> Option<Node<'_>> {
    first_child_kind(property_node, "type_annotation")
}

/// Return the bound name of a Swift property (`let x` / `var x = ...`). Mirrors
/// `_swift_property_name`.
#[must_use]
pub(crate) fn swift_property_name(property_node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = property_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "pattern"
                && let Some(id) = first_child_kind(c, "simple_identifier")
            {
                return Some(read_text_owned(id, source));
            }
            if c.kind() == "simple_identifier" {
                return Some(read_text_owned(c, source));
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// If a Swift call expression is a constructor (`Foo()`), return the type name.
/// Only upper-cased callees are treated as types so a free-function call like
/// `configure()` in an initializer is not mistaken for a constructor. Mirrors
/// `_swift_constructor_type`.
#[must_use]
pub(crate) fn swift_constructor_type(call_node: Node<'_>, source: &[u8]) -> Option<String> {
    let first = call_node.child(0)?;
    if first.kind() == "simple_identifier" {
        let text = read_text_owned(first, source);
        if text.chars().next().is_some_and(char::is_uppercase) {
            return Some(text);
        }
    }
    None
}

/// Walk a Swift type expression; append `(name, role)` tuples. Mirrors
/// Python `_swift_collect_type_refs`.
pub(crate) fn swift_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "user_type" => {
            if let Some(head) = first_child_kind(node, "type_identifier") {
                let text = read_text_owned(head, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        recurse_named_refs(cur.node(), source, true, out, swift_collect_type_refs);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        // `optional_type`, `array_type`, `dictionary_type`, `tuple_type`, etc.
        // are all named wrappers handled identically by the fallback below.
        _ if node.is_named() => {
            recurse_named_refs(node, source, generic, out, swift_collect_type_refs);
        }
        _ => {}
    }
}
