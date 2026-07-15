//! C# type-reference and attribute collectors.

use std::collections::HashSet;

use tree_sitter::Node;

use super::RefRole;
use crate::generic::names::read_text_owned;

/// A collected C# type reference: the simple `name`, its `role` (direct use vs
/// generic argument), and whether the source wrote it `qualified` (`N.T`) with
/// its `qualifier` prefix. Mirrors graphify-py `_csharp_collect_type_refs`'s
/// `(name, role, qualified, qualifier)` yield (#1562).
pub(crate) struct CsharpTypeRef {
    pub name: String,
    pub role: RefRole,
    pub qualified: bool,
    pub qualifier: String,
}

/// Split a dotted type text into `(qualifier, tail)`; `qualifier` is empty when
/// there is no `.`.
fn split_qualified(full: &str) -> (String, String) {
    full.rsplit_once('.').map_or_else(
        || (String::new(), full.to_string()),
        |(p, t)| (p.to_string(), t.to_string()),
    )
}

/// C# declaration node kinds that can declare `<T>` type parameters. Mirrors
/// graphify-py `_CSHARP_TYPE_PARAMETER_SCOPE_DECLARATIONS`.
const CSHARP_TYPE_PARAM_SCOPES: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "record_declaration",
    "struct_declaration",
    "method_declaration",
];

/// Append the `<T>`/`<U>` names declared by a `type_parameter_list`.
fn collect_type_param_names(list: Node<'_>, source: &[u8], names: &mut HashSet<String>) {
    let mut pc = list.walk();
    if !pc.goto_first_child() {
        return;
    }
    loop {
        let param = pc.node();
        if param.kind() == "type_parameter" {
            let mut ic = param.walk();
            if ic.goto_first_child() {
                loop {
                    if ic.node().kind() == "identifier" {
                        let n = read_text_owned(ic.node(), source);
                        if !n.is_empty() {
                            names.insert(n);
                        }
                        break;
                    }
                    if !ic.goto_next_sibling() {
                        break;
                    }
                }
            }
        } else if param.kind() == "identifier" {
            let n = read_text_owned(param, source);
            if !n.is_empty() {
                names.insert(n);
            }
        }
        if !pc.goto_next_sibling() {
            break;
        }
    }
}

/// C# type-parameter names visible from `node` — the `<T>`/`<U>` declared on any
/// enclosing class/interface/record/struct/method. A reference to one is a type
/// variable, not a real type (#1562). Mirrors `_csharp_type_parameters_in_scope`.
#[must_use]
pub(crate) fn csharp_type_parameters_in_scope(node: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut scope = Some(node);
    while let Some(s) = scope {
        if CSHARP_TYPE_PARAM_SCOPES.contains(&s.kind()) {
            let mut cur = s.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_parameter_list" {
                        collect_type_param_names(cur.node(), source, &mut names);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        scope = s.parent();
    }
    names
}

/// Collect C# type references under `node`, skipping any name that is a type
/// parameter in scope. See [`CsharpTypeRef`].
pub(crate) fn csharp_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<CsharpTypeRef>,
) {
    let skip = csharp_type_parameters_in_scope(node, source);
    csharp_collect_type_refs_inner(node, source, generic, &skip, out);
}

// `too_many_lines`: a single linear tree-sitter node dispatch reads clearer than
// fragmenting it. `similar_names`: `qualified` (bool flag) vs `qualifier` (prefix
// string) are distinct fields; renaming either would obscure the flag/prefix roles.
#[allow(clippy::too_many_lines, clippy::similar_names)]
fn csharp_collect_type_refs_inner(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    skip: &HashSet<String>,
    out: &mut Vec<CsharpTypeRef>,
) {
    let role = if generic {
        RefRole::Generic
    } else {
        RefRole::Direct
    };
    let t = node.kind();
    if t == "predefined_type" {
        return;
    }
    if t == "identifier" {
        let name = read_text_owned(node, source);
        if !name.is_empty() && !skip.contains(&name) {
            out.push(CsharpTypeRef {
                name,
                role,
                qualified: false,
                qualifier: String::new(),
            });
        }
        return;
    }
    if t == "qualified_name" {
        let full = read_text_owned(node, source);
        let (qualifier, tail) = split_qualified(&full);
        if !tail.is_empty() && !skip.contains(&tail) {
            out.push(CsharpTypeRef {
                name: tail,
                role,
                qualified: true,
                qualifier,
            });
        }
        return;
    }
    if t == "generic_name" {
        let name_node = node.child_by_field_name("name").or_else(|| {
            let mut sc = node.walk();
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
            let qualified = nn.kind() == "qualified_name";
            let full = read_text_owned(nn, source);
            let (qualifier, tail) = split_qualified(&full);
            if !tail.is_empty() && !skip.contains(&tail) {
                out.push(CsharpTypeRef {
                    name: tail,
                    role,
                    qualified,
                    qualifier: if qualified { qualifier } else { String::new() },
                });
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
                                csharp_collect_type_refs_inner(
                                    acur.node(),
                                    source,
                                    true,
                                    skip,
                                    out,
                                );
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
                    csharp_collect_type_refs_inner(cur.node(), source, generic, skip, out);
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
                    csharp_collect_type_refs_inner(cur.node(), source, generic, skip, out);
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
// `qualified` (bool flag) vs `qualifier` (prefix string) are distinct fields;
// renaming either to satisfy the lint would obscure the flag/prefix roles.
#[allow(clippy::similar_names)]
pub(crate) fn csharp_attribute_names(method_node: Node<'_>, source: &[u8]) -> Vec<CsharpTypeRef> {
    let mut out = Vec::new();
    let mut cur = method_node.walk();
    if !cur.goto_first_child() {
        return out;
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
                            let qualified = nn.kind() == "qualified_name";
                            let text = read_text_owned(nn, source);
                            let (qualifier, tail) = split_qualified(&text);
                            if !tail.is_empty() {
                                out.push(CsharpTypeRef {
                                    name: tail,
                                    role: RefRole::Direct,
                                    qualified,
                                    qualifier: if qualified { qualifier } else { String::new() },
                                });
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
    out
}
