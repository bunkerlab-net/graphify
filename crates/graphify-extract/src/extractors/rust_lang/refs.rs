//! Rust type-reference name collectors.

use super::read_text;

/// Walk a Rust type expression, appending `(name, is_generic_arg)` tuples for
/// each user-defined type referenced. Primitive types are skipped. Mirrors
/// Python `_rust_collect_type_refs`.
pub(super) fn rust_collect_type_refs(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    match node.kind() {
        "primitive_type" => {}
        "type_identifier" => {
            let text = read_text(node, source);
            if !text.is_empty() {
                out.push((text.to_string(), generic));
            }
        }
        "scoped_type_identifier" => {
            let full = read_text(node, source);
            let text = full.rsplit("::").next().unwrap_or(full);
            if !text.is_empty() {
                out.push((text.to_string(), generic));
            }
        }
        "generic_type" => {
            let name_node = node.child_by_field_name("type").or_else(|| {
                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        if matches!(
                            c.node().kind(),
                            "type_identifier" | "scoped_type_identifier"
                        ) {
                            return Some(c.node());
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                None
            });
            if let Some(nn) = name_node {
                let full = read_text(nn, source);
                let text = full.rsplit("::").next().unwrap_or(full);
                if !text.is_empty() {
                    out.push((text.to_string(), generic));
                }
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        let mut acur = cur.node().walk();
                        if acur.goto_first_child() {
                            loop {
                                if acur.node().is_named() {
                                    rust_collect_type_refs(acur.node(), source, true, out);
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
        }
        "reference_type" | "pointer_type" | "array_type" | "tuple_type" | "slice_type" => {
            rust_recurse_named(node, source, generic, out);
        }
        _ if node.is_named() => rust_recurse_named(node, source, generic, out),
        _ => {}
    }
}

/// Recurse `rust_collect_type_refs` over every named child of `node`.
fn rust_recurse_named(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().is_named() {
                rust_collect_type_refs(cur.node(), source, generic, out);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
