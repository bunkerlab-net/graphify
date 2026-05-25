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
pub fn csharp_pre_scan_interfaces(root: Node<'_>, source: &[u8]) -> HashSet<String> {
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

// ── Java ──────────────────────────────────────────────────────────────────────

/// Emit `inherits` and `implements` edges for a Java class or interface node.
///
/// Java's source-level `extends` keyword (class extending a superclass or
/// interface extending other interfaces) is normalised to the `inherits`
/// relation so cross-language consumers see the same shape as C#, Swift, and
/// C++. `implements` (class implementing an interface) is kept as-is. All
/// three cases are handled here to match Python `_extract_java`.
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
                        "inherits",
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
                                                "inherits",
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

// ── TypeScript / JavaScript ──────────────────────────────────────────────────

/// Emit `inherits` / `implements` edges for a TS class declaration's
/// `class_heritage` child.
///
/// TS distinguishes `extends_clause` (single class) from `implements_clause`
/// (one or more interfaces). `extends` is normalised to `inherits` so all
/// languages share a single relation name for class extension. The `name`
/// field's type-arguments are NOT walked here — that happens in the field /
/// method passes via `ts_collect_type_refs`.
pub(super) fn emit_ts_inheritance(
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
        if child.kind() == "class_heritage" {
            let mut hcur = child.walk();
            if hcur.goto_first_child() {
                loop {
                    let clause = hcur.node();
                    let relation = match clause.kind() {
                        "extends_clause" => Some("inherits"),
                        "implements_clause" => Some("implements"),
                        _ => None,
                    };
                    if let Some(rel) = relation {
                        for name in super::references::ts_heritage_clause_entries(clause, source) {
                            let base_nid =
                                emit_base_node(&name, line, stem, str_path, nodes, seen_ids);
                            add_edge(class_nid, &base_nid, rel, line, str_path, None, edges);
                        }
                    }
                    if !hcur.goto_next_sibling() {
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
