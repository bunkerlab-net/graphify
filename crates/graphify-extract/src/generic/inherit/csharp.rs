//! C# inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::names::{read_csharp_type_name, read_text_owned};
use crate::generic::references::{CsharpTypeRef, csharp_collect_type_refs};
use crate::generic::walk::add_edge_meta;

/// Build C# type-reference edge metadata `{ref_token, [qualified], [ref_qualifier]}`
/// (sanitised), or `None` (#1562).
// `qualified` (bool flag) and `qualifier` (prefix string) are distinct fields;
// renaming either to satisfy the lint would obscure the flag/prefix roles.
#[allow(clippy::similar_names)]
fn cs_ref_meta(
    token: &str,
    qualified: bool,
    qualifier: &str,
) -> Option<indexmap::IndexMap<String, serde_json::Value>> {
    use serde_json::Value;
    let mut pairs: Vec<(&str, Value)> = vec![("ref_token", Value::String(token.to_string()))];
    if qualified {
        pairs.push(("qualified", Value::Bool(true)));
    }
    if !qualifier.is_empty() {
        pairs.push(("ref_qualifier", Value::String(qualifier.to_string())));
    }
    crate::generic::walk::sanitized_metadata(pairs)
}
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
    // A base that names an in-scope type parameter (`class Box<T> : T`) is a type
    // variable, not a real base — skip it (#1562).
    let type_params = crate::generic::references::csharp_type_parameters_in_scope(node, source);
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
                    if let Some(info) = read_csharp_type_name(Some(sub), source)
                        && !info.name.is_empty()
                        && !type_params.contains(&info.name)
                    {
                        let base_nid =
                            emit_base_node(&info.name, line, stem, str_path, nodes, seen_ids);
                        let relation = csharp_classify_base(&info.name, interface_names);
                        add_edge_meta(
                            class_nid,
                            &base_nid,
                            relation,
                            line,
                            str_path,
                            None,
                            cs_ref_meta(&info.name, info.qualified, &info.qualifier),
                            edges,
                        );
                        if sub.kind() == "generic_name" {
                            let mut tc = sub.walk();
                            if tc.goto_first_child() {
                                loop {
                                    if tc.node().kind() == "type_argument_list" {
                                        let mut acur = tc.node().walk();
                                        if acur.goto_first_child() {
                                            loop {
                                                if acur.node().is_named() {
                                                    let mut refs: Vec<CsharpTypeRef> = Vec::new();
                                                    csharp_collect_type_refs(
                                                        acur.node(),
                                                        source,
                                                        true,
                                                        &mut refs,
                                                    );
                                                    for r in &refs {
                                                        let target = emit_base_node(
                                                            &r.name, line, stem, str_path, nodes,
                                                            seen_ids,
                                                        );
                                                        // `class Foo : Base<Foo>`
                                                        // yields a generic arg Foo;
                                                        // skip the self-reference
                                                        // (not a real edge Foo->Foo).
                                                        if target != class_nid {
                                                            add_edge_meta(
                                                                class_nid,
                                                                &target,
                                                                "references",
                                                                line,
                                                                str_path,
                                                                Some("generic_arg"),
                                                                cs_ref_meta(
                                                                    &r.name,
                                                                    r.qualified,
                                                                    &r.qualifier,
                                                                ),
                                                                edges,
                                                            );
                                                        }
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
