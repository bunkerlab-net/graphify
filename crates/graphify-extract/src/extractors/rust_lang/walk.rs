//! Rust structural AST walk + type-reference edge emitters.

use super::read_text;
use super::refs::rust_collect_type_refs;
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, Node};
use std::collections::HashSet;

/// Recursively walk a Rust AST emitting nodes for functions, structs, enums, traits, and impls.
///
/// Records function body byte ranges for the subsequent call-graph pass. Handles `use_declaration`
/// to produce import edges. Mirrors Python `_walk_rust`.
/// Shared state threaded through every [`walk_rust`] recursion.
pub(super) struct RustWalkCtx<'a> {
    pub(super) str_path: &'a str,
    pub(super) stem: &'a str,
    pub(super) file_nid: &'a str,
    pub(super) nodes: &'a mut Vec<Node>,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_ids: &'a mut HashSet<String>,
    pub(super) function_bodies: &'a mut Vec<(String, usize, usize)>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Rust's AST node kinds
pub(super) fn walk_rust(
    ctx: &mut RustWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    parent_impl_nid: Option<&str>,
) {
    let t = node.kind();

    match t {
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let func_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let (func_nid, label, parent) = if let Some(impl_nid) = parent_impl_nid {
                    (
                        make_id(&[impl_nid, func_name]),
                        format!(".{func_name}()"),
                        impl_nid.to_string(),
                    )
                } else {
                    (
                        make_id(&[ctx.stem, func_name]),
                        format!("{func_name}()"),
                        ctx.file_nid.to_string(),
                    )
                };
                let relation = if parent_impl_nid.is_some() {
                    "method"
                } else {
                    "contains"
                };
                if ctx.seen_ids.insert(func_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: func_nid.clone(),
                        label,
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: parent,
                    target: func_nid.clone(),
                    relation: relation.to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                emit_rust_param_return_refs(ctx, node, &func_nid, line, source);
                if let Some(body) = node.child_by_field_name("body") {
                    ctx.function_bodies
                        .push((func_nid, body.start_byte(), body.end_byte()));
                }
            }
        }
        "struct_item" | "enum_item" | "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let item_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let item_nid = make_id(&[ctx.stem, item_name]);
                if ctx.seen_ids.insert(item_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: item_nid.clone(),
                        label: item_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: ctx.file_nid.to_string(),
                    target: item_nid.clone(),
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                if t == "trait_item" {
                    emit_rust_trait_bounds(ctx, node, &item_nid, line, source);
                }
                if t == "struct_item" {
                    emit_rust_struct_fields(ctx, node, &item_nid, source);
                }
            }
        }
        "impl_item" => {
            let line = node.start_position().row + 1;
            let mut impl_nid: Option<String> = None;
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = read_text(type_node, source).trim().to_string();
                let nid = make_id(&[ctx.stem, &type_name]);
                if ctx.seen_ids.insert(nid.clone()) {
                    ctx.nodes.push(Node {
                        id: nid.clone(),
                        label: type_name,
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                impl_nid = Some(nid);
            }
            if let (Some(trait_node), Some(inid)) =
                (node.child_by_field_name("trait"), impl_nid.clone())
            {
                emit_rust_impl_trait(ctx, trait_node, &inid, line, source);
            }
            if let Some(body) = node.child_by_field_name("body") {
                let mut cur = body.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_rust(ctx, cur.node(), source, impl_nid.as_deref());
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "use_declaration" => {
            if let Some(arg) = node.child_by_field_name("argument") {
                let raw = read_text(arg, source);
                let clean = raw
                    .split('{')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(':')
                    .trim_end_matches('*')
                    .trim_end_matches(':')
                    .to_string();
                // Strip any `as` alias (`use foo::bar as baz` -> `bar`). Diverges
                // from graphify-py (extract.py:6813), which keeps `bar as baz`.
                let base = clean.split_once(" as ").map_or(clean.as_str(), |(b, _)| b);
                let module_name = base.split("::").last().unwrap_or("").trim().to_string();
                if !module_name.is_empty() {
                    let tgt_nid = make_id1(&module_name);
                    let line = node.start_position().row + 1;
                    ctx.edges.push(Edge {
                        external: false,
                        source: ctx.file_nid.to_string(),
                        target: tgt_nid,
                        relation: "imports_from".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: Some("import".to_string()),
                        confidence_score: None,
                    });
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_rust(
                        ctx,
                        cur.node(),
                        source, // Don't propagate impl_nid through generic ctx.nodes
                        None,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

impl RustWalkCtx<'_> {
    /// Return the NID for a named type, creating a bare placeholder node when no
    /// file-qualified node already exists. Mirrors Rust's `ensure_named_node`.
    fn ensure_named_node(&mut self, name: &str, line: usize) -> String {
        let nid1 = make_id(&[self.stem, name]);
        if self.seen_ids.contains(&nid1) {
            return nid1;
        }
        let nid2 = make_id1(name);
        if self.seen_ids.insert(nid2.clone()) {
            self.nodes.push(Node {
                id: nid2.clone(),
                label: name.to_string(),
                file_type: "code".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
            });
        }
        nid2
    }

    /// Push a `references` edge with the given context.
    fn push_ref(&mut self, src: &str, tgt: &str, context: &str, line: usize) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: "references".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: Some(context.to_string()),
            confidence_score: None,
        });
    }

    /// Push a plain relation edge (e.g. `inherits`, `implements`).
    fn push_rel(&mut self, src: &str, tgt: &str, relation: &str, line: usize) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    }
}

/// Emit `parameter_type` / `return_type` / `generic_arg` references for a `fn`.
fn emit_rust_param_return_refs(
    ctx: &mut RustWalkCtx<'_>,
    func_node: tree_sitter::Node<'_>,
    func_nid: &str,
    line: usize,
    source: &[u8],
) {
    if let Some(params) = func_node.child_by_field_name("parameters") {
        let mut cur = params.walk();
        if cur.goto_first_child() {
            loop {
                let p = cur.node();
                if p.kind() == "parameter"
                    && let Some(type_node) = p.child_by_field_name("type")
                {
                    let mut refs: Vec<(String, bool)> = Vec::new();
                    rust_collect_type_refs(type_node, source, false, &mut refs);
                    for (ref_name, is_generic) in refs {
                        let context = if is_generic {
                            "generic_arg"
                        } else {
                            "parameter_type"
                        };
                        let tgt = ctx.ensure_named_node(&ref_name, line);
                        if tgt != func_nid {
                            ctx.push_ref(func_nid, &tgt, context, line);
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    if let Some(return_type) = func_node.child_by_field_name("return_type") {
        let mut refs: Vec<(String, bool)> = Vec::new();
        rust_collect_type_refs(return_type, source, false, &mut refs);
        for (ref_name, is_generic) in refs {
            let context = if is_generic {
                "generic_arg"
            } else {
                "return_type"
            };
            let tgt = ctx.ensure_named_node(&ref_name, line);
            if tgt != func_nid {
                ctx.push_ref(func_nid, &tgt, context, line);
            }
        }
    }
}

/// Emit supertrait edges from a `trait_item`'s `trait_bounds`.
///
/// Each bound is processed independently: within one bound the leading type is
/// the supertrait (`inherits`) and any following types are its generic
/// arguments (`references[generic_arg]`). So `trait C: A + B` emits `inherits`
/// for both `A` and `B`, while `trait C: Foo<Bar>` emits `inherits C→Foo` and
/// `references[generic_arg] C→Bar`. Mirrors graphify-py's per-bound `enumerate`.
fn emit_rust_trait_bounds(
    ctx: &mut RustWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    item_nid: &str,
    line: usize,
    source: &[u8],
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        if cur.node().kind() == "trait_bounds" {
            let mut bcur = cur.node().walk();
            if bcur.goto_first_child() {
                loop {
                    if bcur.node().is_named() {
                        let mut refs: Vec<(String, bool)> = Vec::new();
                        rust_collect_type_refs(bcur.node(), source, false, &mut refs);
                        for (idx, (ref_name, _is_generic)) in refs.into_iter().enumerate() {
                            let tgt = ctx.ensure_named_node(&ref_name, line);
                            if tgt == item_nid {
                                continue;
                            }
                            if idx == 0 {
                                ctx.push_rel(item_nid, &tgt, "inherits", line);
                            } else {
                                ctx.push_ref(item_nid, &tgt, "generic_arg", line);
                            }
                        }
                    }
                    if !bcur.goto_next_sibling() {
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

/// Emit `references[field]` / `references[generic_arg]` edges from a
/// `struct_item`'s field declarations.
fn emit_rust_struct_fields(
    ctx: &mut RustWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    item_nid: &str,
    source: &[u8],
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        if cur.node().kind() == "field_declaration_list" {
            let mut fcur = cur.node().walk();
            if fcur.goto_first_child() {
                loop {
                    if fcur.node().kind() == "field_declaration" {
                        emit_rust_struct_field(ctx, fcur.node(), item_nid, source);
                    }
                    if !fcur.goto_next_sibling() {
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

/// Emit references for a single Rust struct `field_declaration`.
fn emit_rust_struct_field(
    ctx: &mut RustWalkCtx<'_>,
    field: tree_sitter::Node<'_>,
    item_nid: &str,
    source: &[u8],
) {
    let line = field.start_position().row + 1;
    let type_node = field.child_by_field_name("type").or_else(|| {
        let mut c = field.walk();
        if c.goto_first_child() {
            loop {
                if matches!(
                    c.node().kind(),
                    "type_identifier"
                        | "generic_type"
                        | "scoped_type_identifier"
                        | "reference_type"
                        | "primitive_type"
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
    let Some(type_node) = type_node else {
        return;
    };
    let mut refs: Vec<(String, bool)> = Vec::new();
    rust_collect_type_refs(type_node, source, false, &mut refs);
    for (ref_name, is_generic) in refs {
        let context = if is_generic { "generic_arg" } else { "field" };
        let tgt = ctx.ensure_named_node(&ref_name, line);
        if tgt != item_nid {
            ctx.push_ref(item_nid, &tgt, context, line);
        }
    }
}

/// Emit `implements` (first trait) / `references[generic_arg]` edges from an
/// `impl Trait for Type` block's `trait` node.
fn emit_rust_impl_trait(
    ctx: &mut RustWalkCtx<'_>,
    trait_node: tree_sitter::Node<'_>,
    impl_nid: &str,
    line: usize,
    source: &[u8],
) {
    let mut refs: Vec<(String, bool)> = Vec::new();
    rust_collect_type_refs(trait_node, source, false, &mut refs);
    for (idx, (ref_name, _is_generic)) in refs.into_iter().enumerate() {
        let tgt = ctx.ensure_named_node(&ref_name, line);
        if tgt == impl_nid {
            continue;
        }
        if idx == 0 {
            ctx.push_rel(impl_nid, &tgt, "implements", line);
        } else {
            ctx.push_ref(impl_nid, &tgt, "generic_arg", line);
        }
    }
}
