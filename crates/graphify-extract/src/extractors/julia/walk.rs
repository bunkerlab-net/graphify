//! Julia structural AST walk (modules, structs, functions, imports).

use super::read_text;
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, Node};
use std::collections::HashSet;

/// Extract the function name from a Julia function signature node.
///
/// Handles both simple `function foo(...)` and `foo(...)::ReturnType` signatures by looking
/// for a `call_expression` child whose callee is an `identifier`.
fn func_name_from_signature(sig_node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = sig_node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "call_expression" {
                let callee = {
                    let mut c = child.walk();
                    c.goto_first_child().then(|| c.node())
                };
                if let Some(callee_node) = callee
                    && callee_node.kind() == "identifier"
                {
                    return Some(read_text(callee_node, source).to_string());
                }
                // fallback: first identifier child of call_expression
                let mut c2 = child.walk();
                if c2.goto_first_child() {
                    loop {
                        if c2.node().kind() == "identifier" {
                            return Some(read_text(c2.node(), source).to_string());
                        }
                        if !c2.goto_next_sibling() {
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
    None
}

/// Recursively walk a Julia AST emitting nodes for modules, structs, and functions.
///
/// Handles `module_definition`, `struct_definition`, `function_definition`, `macro_definition`,
/// and `import_statement`/`using_statement`. Mirrors Python `_walk_julia`.
/// Shared state threaded through every [`walk_julia`] recursion.
pub(super) struct JuliaWalkCtx<'a> {
    pub(super) str_path: &'a str,
    pub(super) stem: &'a str,
    pub(super) file_nid: &'a str,
    pub(super) nodes: &'a mut Vec<Node>,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_ids: &'a mut HashSet<String>,
    pub(super) function_bodies: &'a mut Vec<(String, usize, usize, bool)>,
}

impl JuliaWalkCtx<'_> {
    /// Return the NID for a named type, creating a bare placeholder node when no
    /// file-qualified node already exists. Mirrors Julia's `ensure_named_node`.
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
                origin_file: None,
            });
        }
        nid2
    }
}

/// Emit `references[field]` edges for a Julia `struct_definition`'s fields.
///
/// Each `name::Type` field lowers to a `typed_expression` child whose last
/// identifier is the field type. Mirrors the field handling added to Python
/// `extract_julia`.
fn emit_julia_struct_fields(
    ctx: &mut JuliaWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    struct_nid: &str,
    source: &[u8],
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "typed_expression" {
            let mut type_ids: Vec<tree_sitter::Node<'_>> = Vec::new();
            let mut tc = child.walk();
            if tc.goto_first_child() {
                loop {
                    if tc.node().kind() == "identifier" {
                        type_ids.push(tc.node());
                    }
                    if !tc.goto_next_sibling() {
                        break;
                    }
                }
            }
            // A `typed_expression` with at least two identifiers is `name::Type`
            // (or `name::Mod.Type`); the trailing identifier is the field type.
            // `split_last` binds it without a redundant length-then-`last` guard
            // and without an indexing panic path.
            if let Some((last, rest)) = type_ids.split_last()
                && !rest.is_empty()
            {
                let field_line = child.start_position().row + 1;
                let type_name = read_text(*last, source).to_string();
                let type_nid = ctx.ensure_named_node(&type_name, field_line);
                ctx.edges.push(Edge {
                    external: false,
                    source: struct_nid.to_string(),
                    target: type_nid,
                    relation: "references".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{field_line}")),
                    weight: 1.0,
                    context: Some("field".to_string()),
                    confidence_score: None,
                });
            }
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

#[allow(clippy::too_many_lines)] // linear dispatch over Julia's AST node kinds
pub(super) fn walk_julia(
    ctx: &mut JuliaWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    scope_nid: &str,
) {
    let t = node.kind();

    match t {
        "module_definition" => {
            let name_node = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut found = None;
                    loop {
                        if cur.node().kind() == "identifier" {
                            found = Some(cur.node());
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                    found
                } else {
                    None
                }
            };
            if let Some(nn) = name_node {
                let mod_name = read_text(nn, source);
                let mod_nid = make_id(&[ctx.stem, mod_name]);
                let line = node.start_position().row + 1;
                if ctx.seen_ids.insert(mod_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: mod_nid.clone(),
                        label: mod_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                        origin_file: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: ctx.file_nid.to_string(),
                    target: mod_nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_julia(ctx, cur.node(), source, &mod_nid);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "struct_definition" => {
            let type_head = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut found = None;
                    loop {
                        if cur.node().kind() == "type_head" {
                            found = Some(cur.node());
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                    found
                } else {
                    None
                }
            };
            if let Some(th) = type_head {
                let mut bin_expr: Option<tree_sitter::Node<'_>> = None;
                let mut c = th.walk();
                if c.goto_first_child() {
                    loop {
                        if c.node().kind() == "binary_expression" {
                            bin_expr = Some(c.node());
                            break;
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                let line = node.start_position().row + 1;
                if let Some(be) = bin_expr {
                    let identifiers: Vec<tree_sitter::Node<'_>> = {
                        let mut ids = vec![];
                        let mut bc = be.walk();
                        if bc.goto_first_child() {
                            loop {
                                if bc.node().kind() == "identifier" {
                                    ids.push(bc.node());
                                }
                                if !bc.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                        ids
                    };
                    if let Some(first) = identifiers.first() {
                        let struct_name = read_text(*first, source);
                        let struct_nid = make_id(&[ctx.stem, struct_name]);
                        if ctx.seen_ids.insert(struct_nid.clone()) {
                            ctx.nodes.push(Node {
                                id: struct_nid.clone(),
                                label: struct_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: ctx.str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                metadata: None,
                                origin_file: None,
                            });
                        }
                        ctx.edges.push(Edge {
                            external: false,
                            source: scope_nid.to_string(),
                            target: struct_nid.clone(),
                            relation: "defines".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        if identifiers.len() >= 2 {
                            let super_name = read_text(identifiers[identifiers.len() - 1], source);
                            let super_nid = ctx.ensure_named_node(super_name, line);
                            ctx.edges.push(Edge {
                                external: false,
                                source: struct_nid.clone(),
                                target: super_nid,
                                relation: "inherits".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: ctx.str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: None,
                                confidence_score: None,
                            });
                        }
                        emit_julia_struct_fields(ctx, node, &struct_nid, source);
                    }
                } else {
                    let name_node = {
                        let mut cc = th.walk();
                        if cc.goto_first_child() {
                            let mut f = None;
                            loop {
                                if cc.node().kind() == "identifier" {
                                    f = Some(cc.node());
                                    break;
                                }
                                if !cc.goto_next_sibling() {
                                    break;
                                }
                            }
                            f
                        } else {
                            None
                        }
                    };
                    if let Some(nn) = name_node {
                        let struct_name = read_text(nn, source);
                        let struct_nid = make_id(&[ctx.stem, struct_name]);
                        if ctx.seen_ids.insert(struct_nid.clone()) {
                            ctx.nodes.push(Node {
                                id: struct_nid.clone(),
                                label: struct_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: ctx.str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                metadata: None,
                                origin_file: None,
                            });
                        }
                        ctx.edges.push(Edge {
                            external: false,
                            source: scope_nid.to_string(),
                            target: struct_nid.clone(),
                            relation: "defines".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        emit_julia_struct_fields(ctx, node, &struct_nid, source);
                    }
                }
            }
        }
        "abstract_definition" => {
            let type_head = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "type_head" {
                            f = Some(cur.node());
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                    f
                } else {
                    None
                }
            };
            if let Some(th) = type_head {
                let name_node = {
                    let mut cc = th.walk();
                    if cc.goto_first_child() {
                        let mut f = None;
                        loop {
                            if cc.node().kind() == "identifier" {
                                f = Some(cc.node());
                                break;
                            }
                            if !cc.goto_next_sibling() {
                                break;
                            }
                        }
                        f
                    } else {
                        None
                    }
                };
                if let Some(nn) = name_node {
                    let abs_name = read_text(nn, source);
                    let abs_nid = make_id(&[ctx.stem, abs_name]);
                    let line = node.start_position().row + 1;
                    if ctx.seen_ids.insert(abs_nid.clone()) {
                        ctx.nodes.push(Node {
                            id: abs_nid.clone(),
                            label: abs_name.to_string(),
                            file_type: "code".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            metadata: None,
                            origin_file: None,
                        });
                    }
                    ctx.edges.push(Edge {
                        external: false,
                        source: scope_nid.to_string(),
                        target: abs_nid,
                        relation: "defines".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                }
            }
        }
        "function_definition" => {
            let sig_node = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "signature" {
                            f = Some(cur.node());
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                    f
                } else {
                    None
                }
            };
            if let Some(sn) = sig_node
                && let Some(func_name) = func_name_from_signature(sn, source)
            {
                let func_nid = make_id(&[ctx.stem, &func_name]);
                let line = node.start_position().row + 1;
                if ctx.seen_ids.insert(func_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: func_nid.clone(),
                        label: format!("{func_name}()"),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                        origin_file: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: scope_nid.to_string(),
                    target: func_nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                ctx.function_bodies
                    .push((func_nid, node.start_byte(), node.end_byte(), true));
            }
        }
        "assignment" => {
            // Short function: foo(x) = expr
            let lhs = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    Some(cur.node())
                } else {
                    None
                }
            };
            if let Some(lhs_node) = lhs
                && lhs_node.kind() == "call_expression"
                && lhs_node.child_count() > 0
            {
                let callee = {
                    let mut cc = lhs_node.walk();
                    if cc.goto_first_child() {
                        Some(cc.node())
                    } else {
                        None
                    }
                };
                if let Some(callee_node) = callee
                    && callee_node.kind() == "identifier"
                {
                    let func_name = read_text(callee_node, source);
                    let func_nid = make_id(&[ctx.stem, func_name]);
                    let line = node.start_position().row + 1;
                    if ctx.seen_ids.insert(func_nid.clone()) {
                        ctx.nodes.push(Node {
                            id: func_nid.clone(),
                            label: format!("{func_name}()"),
                            file_type: "code".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            metadata: None,
                            origin_file: None,
                        });
                    }
                    ctx.edges.push(Edge {
                        external: false,
                        source: scope_nid.to_string(),
                        target: func_nid.clone(),
                        relation: "defines".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                    // Walk RHS only (last child). tree-sitter 0.26 changed
                    // `child()` to accept `u32`; cast the index explicitly.
                    let count = u32::try_from(node.child_count()).unwrap_or(0);
                    if count >= 3
                        && let Some(rhs) = node.child(count - 1)
                    {
                        ctx.function_bodies.push((
                            func_nid,
                            rhs.start_byte(),
                            rhs.end_byte(),
                            false,
                        ));
                    }
                }
            }
        }
        "using_statement" | "import_statement" => {
            let line = node.start_position().row + 1;
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "identifier" {
                        let mod_name = read_text(child, source);
                        let imp_nid = make_id1(mod_name);
                        ctx.seen_ids.insert(imp_nid.clone());
                        ctx.nodes.push(Node {
                            id: imp_nid.clone(),
                            label: mod_name.to_string(),
                            file_type: "code".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            metadata: None,
                            origin_file: None,
                        });
                        ctx.edges.push(Edge {
                            external: false,
                            source: scope_nid.to_string(),
                            target: imp_nid,
                            relation: "imports".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: Some("import".to_string()),
                            confidence_score: None,
                        });
                    } else if child.kind() == "selected_import" {
                        let idents: Vec<tree_sitter::Node<'_>> = {
                            let mut ids = vec![];
                            let mut sc = child.walk();
                            if sc.goto_first_child() {
                                loop {
                                    if sc.node().kind() == "identifier" {
                                        ids.push(sc.node());
                                    }
                                    if !sc.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                            ids
                        };
                        if let Some(first) = idents.first() {
                            let pkg_name = read_text(*first, source);
                            let pkg_nid = make_id1(pkg_name);
                            ctx.seen_ids.insert(pkg_nid.clone());
                            ctx.nodes.push(Node {
                                id: pkg_nid.clone(),
                                label: pkg_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: ctx.str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                metadata: None,
                                origin_file: None,
                            });
                            ctx.edges.push(Edge {
                                external: false,
                                source: scope_nid.to_string(),
                                target: pkg_nid,
                                relation: "imports".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: ctx.str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: Some("import".to_string()),
                                confidence_score: None,
                            });
                        }
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_julia(ctx, cur.node(), source, scope_nid);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}
