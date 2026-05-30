//! Per-language inheritance-edge emitters.
//!
//! Each `emit_*_inheritance` function is called from the structural `walk`
//! pass when a class node is encountered for the corresponding language.
//! They inspect language-specific child nodes (e.g. `base_list`, `superclass`,
//! `base_class_clause`) and push `inherits` / `extends` / `implements` edges.

// Tree-sitter row numbers are source line indices; files with 2^32+ lines do
// not exist in practice, so usize→u32 truncation is safe.
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashSet;

use tree_sitter::Node;

use crate::types::{Edge, Node as GNode};

use super::names::read_text_owned;
use super::walk::{add_edge, first_child_kind, named_children};

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
            metadata: None,
        });
        seen_ids.insert(nid2.clone());
    }
    nid2
}

// ── Swift ─────────────────────────────────────────────────────────────────────

/// Return the leading kind keyword for a Swift `class_declaration`
/// (`class` / `struct` / `enum` / `extension` / `actor`), if present.
#[must_use]
pub(super) fn swift_declaration_keyword(node: Node<'_>) -> Option<&'static str> {
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
pub(super) fn swift_pre_scan(root: Node<'_>, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
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
pub(super) fn emit_swift_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
) {
    use super::references::{RefRole, swift_collect_type_refs, swift_user_type_name};

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
                                                let target = super::walk::ensure_named_node(
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
pub(super) fn csharp_pre_scan_interfaces(root: Node<'_>, source: &[u8]) -> HashSet<String> {
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

// ── PHP ───────────────────────────────────────────────────────────────────────

/// Emit `inherits` (`extends`) / `implements` (`implements`) / `mixes_in`
/// (trait `use`) edges for a PHP class. Mirrors Python `_extract_php`.
pub(super) fn emit_php_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    _line: u32,
) {
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;

    let emit = |base_name: Option<String>,
                rel: &str,
                at_line: u32,
                nodes: &mut Vec<GNode>,
                edges: &mut Vec<Edge>,
                seen_ids: &mut HashSet<String>| {
        let Some(base_name) = base_name else { return };
        if base_name.is_empty() {
            return;
        }
        let base_nid = emit_base_node(&base_name, at_line, stem, str_path, nodes, seen_ids);
        add_edge(class_nid, &base_nid, rel, at_line, str_path, None, edges);
    };

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            let child_line = child.start_position().row as u32 + 1;
            match child.kind() {
                "base_clause" => {
                    for sub in named_children(child) {
                        if matches!(sub.kind(), "name" | "qualified_name") {
                            emit(
                                super::references::php_name_text(sub, source),
                                "inherits",
                                child_line,
                                nodes,
                                edges,
                                seen_ids,
                            );
                        }
                    }
                }
                "class_interface_clause" => {
                    for sub in named_children(child) {
                        if matches!(sub.kind(), "name" | "qualified_name") {
                            emit(
                                super::references::php_name_text(sub, source),
                                "implements",
                                child_line,
                                nodes,
                                edges,
                                seen_ids,
                            );
                        }
                    }
                }
                _ => {}
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    // Trait `use` declarations inside the class body → `mixes_in`.
    let body = node
        .child_by_field_name("body")
        .or_else(|| first_child_kind(node, "declaration_list"));
    if let Some(body) = body {
        for member in named_children(body) {
            if member.kind() != "use_declaration" {
                continue;
            }
            let member_line = member.start_position().row as u32 + 1;
            for sub in named_children(member) {
                if matches!(sub.kind(), "name" | "qualified_name") {
                    emit(
                        super::references::php_name_text(sub, source),
                        "mixes_in",
                        member_line,
                        nodes,
                        edges,
                        seen_ids,
                    );
                }
            }
        }
    }
}

// ── Kotlin ────────────────────────────────────────────────────────────────────

/// Emit `inherits` (`: Base()`) / `implements` (`: Interface`) edges for a
/// Kotlin class's `delegation_specifiers`, plus `references[generic_arg]` for
/// type arguments on the base. Mirrors Python `_extract_kotlin`.
pub(super) fn emit_kotlin_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
) {
    use super::references::{RefRole, kotlin_collect_type_refs, kotlin_user_type_name};
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;

    for child in named_children(node) {
        if child.kind() != "delegation_specifiers" {
            continue;
        }
        for spec in named_children(child) {
            if spec.kind() != "delegation_specifier" {
                continue;
            }
            let mut relation = "implements";
            let mut user_type_node: Option<Node<'_>> = None;
            for sub in named_children(spec) {
                if sub.kind() == "constructor_invocation" {
                    relation = "inherits";
                    user_type_node = first_child_kind(sub, "user_type");
                    break;
                }
                if sub.kind() == "user_type" {
                    user_type_node = Some(sub);
                    break;
                }
            }
            let Some(ut) = user_type_node else { continue };
            let Some(base) = kotlin_user_type_name(ut, source) else {
                continue;
            };
            let base_nid = emit_base_node(&base, line, stem, str_path, nodes, seen_ids);
            add_edge(class_nid, &base_nid, relation, line, str_path, None, edges);
            for arg_child in named_children(ut) {
                if arg_child.kind() != "type_arguments" {
                    continue;
                }
                for arg in named_children(arg_child) {
                    let mut refs: Vec<(String, RefRole)> = Vec::new();
                    if arg.kind() == "type_projection" {
                        for inner in named_children(arg) {
                            kotlin_collect_type_refs(inner, source, true, &mut refs);
                        }
                    } else {
                        kotlin_collect_type_refs(arg, source, true, &mut refs);
                    }
                    for (ref_name, _role) in refs {
                        let target = super::walk::ensure_named_node(
                            &ref_name, line, stem, str_path, nodes, seen_ids,
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
            }
        }
    }
}

// ── Scala ─────────────────────────────────────────────────────────────────────

/// Emit `inherits` (first base after `extends`) / `mixes_in` (each `with`
/// trait) edges plus `references[field]` edges for constructor parameters.
/// Mirrors Python `_extract_scala`.
pub(super) fn emit_scala_inheritance(
    ctx: &mut super::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    _line: u32,
) {
    use super::references::{RefRole, scala_collect_type_refs};
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;

    let extend = node
        .child_by_field_name("extend")
        .or_else(|| first_child_kind(node, "extends_clause"));
    if let Some(extend) = extend {
        let mut bases: Vec<(String, u32)> = Vec::new();
        for c in named_children(extend) {
            let c_line = c.start_position().row as u32 + 1;
            if c.kind() == "type_identifier" {
                bases.push((read_text_owned(c, source), c_line));
            } else if c.kind() == "generic_type" {
                let base = c
                    .child_by_field_name("type")
                    .or_else(|| first_child_kind(c, "type_identifier"));
                if let Some(base) = base {
                    bases.push((read_text_owned(base, source), c_line));
                }
            }
        }
        for (idx, (base_name, base_line)) in bases.into_iter().enumerate() {
            let rel = if idx == 0 { "inherits" } else { "mixes_in" };
            let base_nid = super::walk::ensure_named_node(
                &base_name, base_line, stem, str_path, nodes, seen_ids,
            );
            if base_nid != class_nid {
                add_edge(class_nid, &base_nid, rel, base_line, str_path, None, edges);
            }
        }
    }

    for c in named_children(node) {
        if c.kind() != "class_parameters" {
            continue;
        }
        for cp in named_children(c) {
            if cp.kind() != "class_parameter" {
                continue;
            }
            let Some(ptype) = cp.child_by_field_name("type") else {
                continue;
            };
            let cp_line = cp.start_position().row as u32 + 1;
            let mut refs: Vec<(String, RefRole)> = Vec::new();
            scala_collect_type_refs(ptype, source, false, &mut refs);
            for (ref_name, role) in refs {
                let context = role.into_context("field");
                let target = super::walk::ensure_named_node(
                    &ref_name, cp_line, stem, str_path, nodes, seen_ids,
                );
                if target != class_nid {
                    add_edge(
                        class_nid,
                        &target,
                        "references",
                        cp_line,
                        str_path,
                        Some(context),
                        edges,
                    );
                }
            }
        }
    }
}
