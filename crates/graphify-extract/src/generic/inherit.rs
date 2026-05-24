//! Per-language inheritance-edge emitters.
//!
//! Each `emit_*_inheritance` function is called from the structural `walk`
//! pass when a class node is encountered for the corresponding language.
//! They inspect language-specific child nodes (e.g. `base_list`, `superclass`,
//! `base_class_clause`) and push `inherits` / `extends` / `implements` edges.

use std::collections::HashSet;

use tree_sitter::Node;

use crate::types::{Edge, Node as GNode};

use super::names::read_text_owned;
use super::walk::add_edge;

// ── Shared helper ─────────────────────────────────────────────────────────────

/// Ensure a base-class node exists and return its NID.
pub(super) fn emit_base_node(
    base: &str,
    _line: u32,
    stem: &str,
    _str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) -> String {
    use crate::ids::{make_id, make_id1};

    let nid1 = make_id(&[stem, base]);
    if seen_ids.contains(&nid1) {
        return nid1;
    }
    let nid2 = make_id1(base);
    if !seen_ids.contains(&nid2) {
        nodes.push(GNode {
            id: nid2.clone(),
            label: base.to_string(),
            file_type: "code".to_string(),
            source_file: String::new(),
            source_location: None,
        });
        seen_ids.insert(nid2.clone());
    }
    nid2
}

// ── Swift ─────────────────────────────────────────────────────────────────────

/// Emit `inherits` edges for Swift class/protocol `inheritance_specifier` nodes.
///
/// Swift uses `inheritance_specifier` children inside the class/protocol body
/// to list both superclasses and protocol conformances; this function treats
/// all of them uniformly as `inherits` edges, matching Python `_extract_swift`.
pub(super) fn emit_swift_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
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
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "inheritance_specifier" {
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    if matches!(sub.kind(), "user_type" | "type_identifier") {
                        let base = read_text_owned(sub, source);
                        let base_nid = emit_base_node(&base, line, stem, str_path, nodes, seen_ids);
                        add_edge(
                            class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
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

// ── C# ────────────────────────────────────────────────────────────────────────

/// Emit `inherits` edges from a C# `base_list` node.
///
/// Both base classes and implemented interfaces appear in the `base_list`,
/// so all entries produce `inherits` edges — the graph does not currently
/// distinguish between extension and implementation for C#.
pub(super) fn emit_csharp_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
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
                        add_edge(
                            class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
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

// ── Java ──────────────────────────────────────────────────────────────────────

/// Emit `extends` and `implements` edges for a Java class or interface node.
///
/// Java distinguishes `extends` (single-class inheritance) from `implements`
/// (interface implementation), and `interface_declaration` nodes use
/// `extends_interfaces` for their own inheritance. All three cases are handled
/// here to match Python `_extract_java`.
#[allow(clippy::too_many_lines)] // sequential dispatch over Java's three inheritance shapes
pub(super) fn emit_java_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    node_type: &str,
    line: u32,
) {
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let emit = |base_name: &str,
                rel: &str,
                nodes: &mut Vec<GNode>,
                edges: &mut Vec<Edge>,
                seen_ids: &mut HashSet<String>| {
        if base_name.is_empty() {
            return;
        }
        let base_nid = emit_base_node(base_name, line, stem, str_path, nodes, seen_ids);
        add_edge(class_nid, &base_nid, rel, line, str_path, None, edges);
    };

    if let Some(sup) = node.child_by_field_name("superclass") {
        let mut cur = sup.walk();
        if cur.goto_first_child() {
            loop {
                let sub = cur.node();
                if sub.kind() == "type_identifier" {
                    emit(
                        &read_text_owned(sub, source),
                        "extends",
                        nodes,
                        edges,
                        seen_ids,
                    );
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    if let Some(ifs) = node.child_by_field_name("interfaces") {
        let mut cur = ifs.walk();
        if cur.goto_first_child() {
            loop {
                let sub = cur.node();
                if sub.kind() == "type_list" {
                    let mut tcur = sub.walk();
                    if tcur.goto_first_child() {
                        loop {
                            let tid = tcur.node();
                            if tid.kind() == "type_identifier" {
                                emit(
                                    &read_text_owned(tid, source),
                                    "implements",
                                    nodes,
                                    edges,
                                    seen_ids,
                                );
                            }
                            if !tcur.goto_next_sibling() {
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

    if node_type == "interface_declaration" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "extends_interfaces" {
                    let mut scur = child.walk();
                    if scur.goto_first_child() {
                        loop {
                            let sub = scur.node();
                            if sub.kind() == "type_list" {
                                let mut tcur = sub.walk();
                                if tcur.goto_first_child() {
                                    loop {
                                        let tid = tcur.node();
                                        if tid.kind() == "type_identifier" {
                                            emit(
                                                &read_text_owned(tid, source),
                                                "extends",
                                                nodes,
                                                edges,
                                                seen_ids,
                                            );
                                        }
                                        if !tcur.goto_next_sibling() {
                                            break;
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
    }
}

// ── C++ ───────────────────────────────────────────────────────────────────────

/// Emit `inherits` edges from a C++ `base_class_clause` node.
///
/// C++ allows multiple inheritance; all entries in the clause produce
/// `inherits` edges regardless of access specifier (`public`, `protected`,
/// `private`), matching Python `_extract_cpp`.
pub(super) fn emit_cpp_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
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
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "base_class_clause" {
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    let base = match sub.kind() {
                        "type_identifier" => Some(read_text_owned(sub, source)),
                        "qualified_identifier" | "template_type" => {
                            if let Some(tail) = sub.child_by_field_name("name") {
                                Some(read_text_owned(tail, source))
                            } else {
                                Some(read_text_owned(sub, source))
                            }
                        }
                        _ => None,
                    };
                    if let Some(b) = base
                        && !b.is_empty()
                    {
                        let base_nid = emit_base_node(&b, line, stem, str_path, nodes, seen_ids);
                        add_edge(
                            class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
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
