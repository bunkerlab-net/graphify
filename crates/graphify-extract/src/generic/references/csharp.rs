//! C# type-reference and attribute collectors.

use tree_sitter::Node;

use super::RefRole;
use crate::generic::names::read_text_owned;

#[allow(clippy::too_many_lines)] // single recursive dispatch over tree-sitter C# type kinds
pub(crate) fn csharp_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    let t = node.kind();
    if t == "predefined_type" {
        return;
    }
    if t == "identifier" {
        let name = read_text_owned(node, source);
        if !name.is_empty() {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((name, role));
        }
        return;
    }
    if t == "qualified_name" {
        let full = read_text_owned(node, source);
        let tail = full.rsplit('.').next().unwrap_or(&full);
        if !tail.is_empty() {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((tail.to_string(), role));
        }
        return;
    }
    if t == "generic_name" {
        let name_node = node.child_by_field_name("name").or_else(|| {
            let mut sc = node.walk();
            if sc.goto_first_child() {
                loop {
                    if sc.node().kind() == "identifier" {
                        return Some(sc.node());
                    }
                    if !sc.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        });
        if let Some(nn) = name_node {
            let name = read_text_owned(nn, source);
            if !name.is_empty() {
                let role = if generic {
                    RefRole::Generic
                } else {
                    RefRole::Direct
                };
                out.push((name, role));
            }
        }
        let mut sc = node.walk();
        if sc.goto_first_child() {
            loop {
                if sc.node().kind() == "type_argument_list" {
                    let mut acur = sc.node().walk();
                    if acur.goto_first_child() {
                        loop {
                            if acur.node().is_named() {
                                csharp_collect_type_refs(acur.node(), source, true, out);
                            }
                            if !acur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                if !sc.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }
    if matches!(
        t,
        "nullable_type" | "array_type" | "pointer_type" | "ref_type"
    ) {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    csharp_collect_type_refs(cur.node(), source, generic, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }
    if node.is_named() {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    csharp_collect_type_refs(cur.node(), source, generic, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Collect attribute names from a C# method's `attribute_list` children.
///
/// `[Authorize, Route("/api")]` on a method produces `["Authorize", "Route"]`.
#[must_use]
pub(crate) fn csharp_attribute_names(method_node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = method_node.walk();
    if !cur.goto_first_child() {
        return names;
    }
    loop {
        let child = cur.node();
        if child.kind() == "attribute_list" {
            let mut acur = child.walk();
            if acur.goto_first_child() {
                loop {
                    let attr = acur.node();
                    if attr.kind() == "attribute" {
                        let name_node = attr.child_by_field_name("name").or_else(|| {
                            let mut sc = attr.walk();
                            if sc.goto_first_child() {
                                loop {
                                    if matches!(sc.node().kind(), "identifier" | "qualified_name") {
                                        return Some(sc.node());
                                    }
                                    if !sc.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                            None
                        });
                        if let Some(nn) = name_node {
                            let text = read_text_owned(nn, source);
                            let tail = text.rsplit('.').next().unwrap_or(&text);
                            if !tail.is_empty() {
                                names.push(tail.to_string());
                            }
                        }
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
    names
}
