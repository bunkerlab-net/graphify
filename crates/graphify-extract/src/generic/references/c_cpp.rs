//! C / C++ type-reference collectors.

use tree_sitter::Node;

use super::{RefRole, recurse_named_refs, role_of};
use crate::generic::names::read_text_owned;

/// Node kinds that are C/C++ primitive types and never yield a type reference.
const C_PRIMITIVE_TYPE_NODES: &[&str] = &[
    "primitive_type",
    "sized_type_specifier",
    "auto",
    "placeholder_type_specifier",
];

/// Walk a C type expression; append `(name, role)` tuples for user-defined types.
pub(crate) fn c_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    if C_PRIMITIVE_TYPE_NODES.contains(&node.kind()) {
        return;
    }
    match node.kind() {
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        "pointer_declarator"
        | "reference_declarator"
        | "array_declarator"
        | "type_qualifier"
        | "type_descriptor"
        | "abstract_pointer_declarator"
        | "abstract_reference_declarator"
        | "abstract_array_declarator" => {
            recurse_named_refs(node, source, generic, out, c_collect_type_refs);
        }
        _ => {}
    }
}

/// Walk a C++ type expression; append `(name, role)` tuples. Resolves
/// `qualified_identifier` tails and `template_type` base + arguments.
pub(crate) fn cpp_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    if C_PRIMITIVE_TYPE_NODES.contains(&node.kind()) {
        return;
    }
    match node.kind() {
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        "qualified_identifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                cpp_collect_type_refs(name_node, source, generic, out);
            }
        }
        "template_type" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let text = read_text_owned(name_node, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            if let Some(args_node) = node.child_by_field_name("arguments") {
                recurse_named_refs(args_node, source, true, out, cpp_collect_type_refs);
            }
        }
        "type_descriptor"
        | "pointer_declarator"
        | "reference_declarator"
        | "array_declarator"
        | "type_qualifier"
        | "abstract_pointer_declarator"
        | "abstract_reference_declarator"
        | "abstract_array_declarator" => {
            recurse_named_refs(node, source, generic, out, cpp_collect_type_refs);
        }
        _ => {}
    }
}
