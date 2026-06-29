//! Swift inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::names::read_text_owned;
use crate::generic::walk::{add_edge, first_child_kind};
use std::collections::HashSet;
use tree_sitter::Node;

/// Return the leading kind keyword for a Swift `class_declaration`
/// (`class` / `struct` / `enum` / `extension` / `actor`), if present.
#[must_use]
pub(crate) fn swift_declaration_keyword(node: Node<'_>) -> Option<&'static str> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if !c.is_named() {
                match c.kind() {
                    "class" => return Some("class"),
                    "struct" => return Some("struct"),
                    "enum" => return Some("enum"),
                    "extension" => return Some("extension"),
                    "actor" => return Some("actor"),
                    _ => {}
                }
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Pre-scan a Swift compilation unit, returning `(protocol_names, class_like_names)`.
///
/// Used to classify each `inheritance_specifier` entry as `inherits` (a class)
/// or `implements` (a protocol). Mirrors Python `_swift_pre_scan`.
#[must_use]
pub(crate) fn swift_pre_scan(root: Node<'_>, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let mut protocols: HashSet<String> = HashSet::new();
    let mut classes: HashSet<String> = HashSet::new();
    let mut stack: Vec<Node<'_>> = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "protocol_declaration" {
            let name_node = n
                .child_by_field_name("name")
                .or_else(|| first_child_kind(n, "type_identifier"));
            if let Some(nn) = name_node {
                let text = read_text_owned(nn, source);
                if !text.is_empty() {
                    protocols.insert(text);
                }
            }
        } else if n.kind() == "class_declaration"
            && matches!(
                swift_declaration_keyword(n),
                Some("class" | "struct" | "enum" | "actor")
            )
            && let Some(nn) = n.child_by_field_name("name")
        {
            let text = read_text_owned(nn, source);
            if !text.is_empty() {
                classes.insert(text);
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
    (protocols, classes)
}

/// Classify a Swift inheritance entry as `inherits` or `implements`.
///
/// Declared protocols → `implements`; declared classes → `inherits`. A
/// `struct`/`enum`/`extension`/`actor` can only conform to protocols, so all
/// of its entries are `implements`. For a `class`, the first entry is the base
/// class (`inherits`) and the rest are protocol conformances (`implements`).
/// Mirrors Python `_swift_classify_base`.
fn swift_classify_base(
    name: &str,
    kind: Option<&str>,
    is_first: bool,
    protocols: &HashSet<String>,
    classes: &HashSet<String>,
) -> &'static str {
    if protocols.contains(name) {
        return "implements";
    }
    if classes.contains(name) {
        return "inherits";
    }
    if matches!(kind, Some("struct" | "enum" | "extension" | "actor")) {
        return "implements";
    }
    if is_first { "inherits" } else { "implements" }
}

/// Emit `inherits` / `implements` edges for a Swift class/protocol/extension's
/// `inheritance_specifier` children, plus `references[generic_arg]` edges for
/// any generic arguments on a base type. Mirrors Python `_extract_swift`.
#[allow(clippy::too_many_lines)] // linear walk over inheritance specifiers + their generic args
pub(crate) fn emit_swift_inheritance(
    ctx: &mut crate::generic::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
) {
    use crate::generic::references::{RefRole, swift_collect_type_refs, swift_user_type_name};

    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let protocols = ctx.swift_protocol_names;
    let classes = ctx.swift_class_names;
    let is_protocol = node.kind() == "protocol_declaration";
    let kind = if node.kind() == "class_declaration" {
        swift_declaration_keyword(node)
    } else {
        Some("protocol")
    };
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;

    let mut seen_base = false;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "inheritance_specifier" {
            // Resolve the base name (and the user_type carrying any generics).
            let mut base_name: Option<String> = None;
            let mut user_type_node: Option<Node<'_>> = None;
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    if sub.kind() == "user_type" {
                        user_type_node = Some(sub);
                        base_name = swift_user_type_name(sub, source);
                        break;
                    }
                    if sub.kind() == "type_identifier" {
                        let t = read_text_owned(sub, source);
                        base_name = (!t.is_empty()).then_some(t);
                        break;
                    }
                    if !scur.goto_next_sibling() {
                        break;
                    }
                }
            }
            if let Some(base_name) = base_name {
                let base_nid = emit_base_node(&base_name, line, stem, str_path, nodes, seen_ids);
                let relation = if is_protocol {
                    "inherits"
                } else {
                    swift_classify_base(&base_name, kind, !seen_base, protocols, classes)
                };
                seen_base = true;
                add_edge(class_nid, &base_nid, relation, line, str_path, None, edges);
                // Generic arguments on the base type → references[generic_arg].
                if let Some(ut) = user_type_node {
                    let mut tacur = ut.walk();
                    if tacur.goto_first_child() {
                        loop {
                            if tacur.node().kind() == "type_arguments" {
                                let mut acur = tacur.node().walk();
                                if acur.goto_first_child() {
                                    loop {
                                        if acur.node().is_named() {
                                            let mut refs: Vec<(String, RefRole)> = Vec::new();
                                            swift_collect_type_refs(
                                                acur.node(),
                                                source,
                                                true,
                                                &mut refs,
                                            );
                                            for (ref_name, _role) in refs {
                                                let target =
                                                    crate::generic::walk::ensure_named_node(
                                                        &ref_name, stem, str_path, nodes, seen_ids,
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
                            if !tacur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
            }
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}
