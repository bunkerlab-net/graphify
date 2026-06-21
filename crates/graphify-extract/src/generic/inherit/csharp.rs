//! C# inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::names::read_text_owned;
use crate::generic::walk::add_edge;
use std::collections::HashSet;
use tree_sitter::Node;

/// Walk the whole tree and return the set of identifiers declared as
/// `interface` in this C# compilation unit.
///
/// Used by [`emit_csharp_inheritance`] to classify each entry in a
/// `base_list`: declared interfaces produce an `implements` edge, everything
/// else falls back to the I-prefix heuristic (`IFoo` with a capital second
/// letter) or is treated as a base class (`inherits`).
///
/// Mirrors Python `_csharp_pre_scan_interfaces`.
#[must_use]
pub(crate) fn csharp_pre_scan_interfaces(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut stack: Vec<Node<'_>> = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "interface_declaration"
            && let Some(name_node) = n.child_by_field_name("name")
        {
            let text = read_text_owned(name_node, source);
            if !text.is_empty() {
                out.insert(text);
            }
        }
        let mut cur = n.walk();
        if cur.goto_first_child() {
            loop {
                stack.push(cur.node());
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    out
}

/// Classify a C# base-list entry as `implements` or `inherits`.
///
/// An entry is `implements` when the name was declared as `interface` in this
/// compilation unit, or when it follows the C# `I<UpperLetter>…` interface
/// naming convention. Otherwise it is `inherits`.
fn csharp_classify_base(name: &str, interface_names: &HashSet<String>) -> &'static str {
    if interface_names.contains(name) {
        return "implements";
    }
    let mut chars = name.chars();
    if let (Some(first), Some(second)) = (chars.next(), chars.next())
        && first == 'I'
        && second.is_uppercase()
    {
        return "implements";
    }
    "inherits"
}

/// Walk a C# type-argument tree and append `(name, role)` tuples where role is
/// `"generic_arg"` for arguments nested inside a `type_argument_list`.
///
/// Mirrors Python `_csharp_collect_type_refs` restricted to the generic case.
fn csharp_collect_type_arg_refs(node: Node<'_>, source: &[u8], out: &mut Vec<String>) {
    let t = node.kind();
    if t == "predefined_type" {
        return;
    }
    if t == "identifier" {
        let name = read_text_owned(node, source);
        if !name.is_empty() {
            out.push(name);
        }
        return;
    }
    if t == "qualified_name" {
        let text = read_text_owned(node, source);
        let tail = text.rsplit('.').next().unwrap_or(&text).to_string();
        if !tail.is_empty() {
            out.push(tail);
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
                out.push(name);
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
                                csharp_collect_type_arg_refs(acur.node(), source, out);
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
                    csharp_collect_type_arg_refs(cur.node(), source, out);
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
                    csharp_collect_type_arg_refs(cur.node(), source, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Emit `inherits` / `implements` edges from a C# `base_list` node.
///
/// Each base-list entry is classified by [`csharp_classify_base`]; declared
/// interfaces (and `I<UpperLetter>…`-named types) produce `implements`,
/// everything else `inherits`. When the entry is a `generic_name`, its type
/// arguments also produce `references` edges with `context = generic_arg` so
/// downstream queries can tell `class Foo : IBar<Baz>` introduces a usage of
/// `Baz`.
pub(crate) fn emit_csharp_inheritance(
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
    let interface_names = ctx.csharp_interface_names;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "base_list" {
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    let base = match sub.kind() {
                        "identifier" => Some(read_text_owned(sub, source)),
                        "qualified_name" => {
                            let full = read_text_owned(sub, source);
                            Some(full.rsplit('.').next().unwrap_or(&full).to_string())
                        }
                        "generic_name" => {
                            if let Some(nc) = sub.child_by_field_name("name") {
                                Some(read_text_owned(nc, source))
                            } else {
                                {
                                    let mut tc = sub.walk();
                                    if tc.goto_first_child() {
                                        Some(tc.node())
                                    } else {
                                        None
                                    }
                                }
                                .map(|first| read_text_owned(first, source))
                            }
                        }
                        _ => None,
                    };
                    if let Some(b) = base
                        && !b.is_empty()
                    {
                        let base_nid = emit_base_node(&b, line, stem, str_path, nodes, seen_ids);
                        let relation = csharp_classify_base(&b, interface_names);
                        add_edge(class_nid, &base_nid, relation, line, str_path, None, edges);
                        if sub.kind() == "generic_name" {
                            let mut tc = sub.walk();
                            if tc.goto_first_child() {
                                loop {
                                    if tc.node().kind() == "type_argument_list" {
                                        let mut acur = tc.node().walk();
                                        if acur.goto_first_child() {
                                            loop {
                                                if acur.node().is_named() {
                                                    let mut refs: Vec<String> = Vec::new();
                                                    csharp_collect_type_arg_refs(
                                                        acur.node(),
                                                        source,
                                                        &mut refs,
                                                    );
                                                    for ref_name in refs {
                                                        let target = emit_base_node(
                                                            &ref_name, line, stem, str_path, nodes,
                                                            seen_ids,
                                                        );
                                                        add_edge(
                                                            class_nid,
                                                            &target,
                                                            "references",
                                                            line,
                                                            str_path,
                                                            Some("generic_arg"),
                                                            edges,
                                                        );
                                                    }
                                                }
                                                if !acur.goto_next_sibling() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if !tc.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                        }
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
