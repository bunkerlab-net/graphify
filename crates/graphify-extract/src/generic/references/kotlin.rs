//! Kotlin type-reference helpers.

use tree_sitter::Node;

use super::{RefRole, recurse_named_refs, role_of};
use crate::generic::names::read_text_owned;

/// Return the head identifier text from a Kotlin `user_type` node.
#[must_use]
pub(crate) fn kotlin_user_type_name(user_type_node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = user_type_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            match c.kind() {
                "type_identifier" | "identifier" => {
                    let text = read_text_owned(c, source);
                    return if text.is_empty() { None } else { Some(text) };
                }
                "simple_user_type" => {
                    if let Some(sub) = first_named_identifier(c) {
                        let text = read_text_owned(sub, source);
                        return if text.is_empty() { None } else { Some(text) };
                    }
                }
                _ => {}
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Return the first `identifier` / `type_identifier` child of `node`.
fn first_named_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if matches!(cur.node().kind(), "identifier" | "type_identifier") {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Find the type node within a Kotlin `property_declaration`.
#[must_use]
pub(crate) fn kotlin_property_type_node(property_node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = property_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "variable_declaration"
                && let Some(sub) = kotlin_type_child(c)
            {
                return Some(sub);
            }
            if matches!(c.kind(), "user_type" | "nullable_type" | "type_reference") {
                return Some(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn kotlin_type_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if matches!(
                cur.node().kind(),
                "user_type" | "nullable_type" | "type_reference"
            ) {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Find the return-type node of a Kotlin `function_declaration`.
#[must_use]
pub(crate) fn kotlin_function_return_type_node(func_node: Node<'_>) -> Option<Node<'_>> {
    let mut saw_params = false;
    let mut saw_colon = false;
    let mut cur = func_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "function_value_parameters" {
                saw_params = true;
            } else if saw_params && c.kind() == ":" {
                saw_colon = true;
            } else if saw_colon && c.is_named() {
                return Some(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Walk a Kotlin type expression; append `(name, role)` tuples. Mirrors
/// Python `_kotlin_collect_type_refs`.
pub(crate) fn kotlin_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "integral_literal" | "boolean_literal" => {}
        "user_type" => {
            if let Some(head) = kotlin_user_type_head(node) {
                let text = read_text_owned(head, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            kotlin_collect_type_arguments(node, source, out);
        }
        "identifier" | "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        // `nullable_type` / `parenthesized_type` / `type_reference` are named
        // wrappers handled identically by the fallback below.
        _ if node.is_named() => {
            recurse_named_refs(node, source, generic, out, kotlin_collect_type_refs);
        }
        _ => {}
    }
}

/// Return the head `identifier`/`type_identifier` node of a Kotlin `user_type`,
/// drilling through a `simple_user_type` wrapper.
fn kotlin_user_type_head(node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if matches!(c.kind(), "identifier" | "type_identifier") {
                return Some(c);
            }
            if c.kind() == "simple_user_type" {
                return first_named_identifier(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Recurse into a Kotlin `user_type`'s `type_arguments`, marking refs generic.
fn kotlin_collect_type_arguments(node: Node<'_>, source: &[u8], out: &mut Vec<(String, RefRole)>) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        if cur.node().kind() == "type_arguments" {
            let mut acur = cur.node().walk();
            if acur.goto_first_child() {
                loop {
                    let arg = acur.node();
                    if arg.kind() == "type_projection" {
                        recurse_named_refs(arg, source, true, out, kotlin_collect_type_refs);
                    } else if arg.is_named() {
                        kotlin_collect_type_refs(arg, source, true, out);
                    }
                    if !acur.goto_next_sibling() {
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
