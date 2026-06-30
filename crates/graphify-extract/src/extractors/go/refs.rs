//! Go type-reference + struct/interface field edge emitters.

use super::read_text;
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, Node};
use std::collections::HashSet;

/// Go's predeclared type identifiers — never emitted as semantic type references.
const GO_PREDECLARED_TYPES: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
    "comparable",
];

/// Walk a Go type expression, appending `(name, is_generic_arg)` tuples for each
/// user-defined type referenced. Predeclared types are skipped. Mirrors Python
/// `_go_collect_type_refs`.
fn go_collect_type_refs(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    match node.kind() {
        "type_identifier" => {
            let text = read_text(node, source);
            if !text.is_empty() && !GO_PREDECLARED_TYPES.contains(&text) {
                out.push((text.to_string(), generic));
            }
        }
        "qualified_type" => {
            let full = read_text(node, source);
            let text = full.rsplit('.').next().unwrap_or(full);
            if !text.is_empty() && !GO_PREDECLARED_TYPES.contains(&text) {
                out.push((text.to_string(), generic));
            }
        }
        "generic_type" => {
            if let Some(type_field) = node.child_by_field_name("type") {
                go_collect_type_refs(type_field, source, generic, out);
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        let mut acur = cur.node().walk();
                        if acur.goto_first_child() {
                            loop {
                                if acur.node().is_named() {
                                    go_collect_type_refs(acur.node(), source, true, out);
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
            }
        }
        "pointer_type" | "slice_type" | "array_type" | "map_type" | "channel_type"
        | "parenthesized_type" => {
            recurse_named_children(node, source, generic, out);
        }
        _ if node.is_named() => recurse_named_children(node, source, generic, out),
        _ => {}
    }
}

/// Recurse `go_collect_type_refs` over every named child of `node`.
fn recurse_named_children(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().is_named() {
                go_collect_type_refs(cur.node(), source, generic, out);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Mutable graph state for the Go semantic-reference passes. Constructed by
/// reborrowing the structural-walk locals at each call site so these passes
/// never need to thread the full [`GoWalkCtx`].
pub(super) struct GoRefCtx<'a> {
    pub(super) source: &'a [u8],
    pub(super) pkg_scope: &'a str,
    pub(super) str_path: &'a str,
    pub(super) nodes: &'a mut Vec<Node>,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_ids: &'a mut HashSet<String>,
}

impl GoRefCtx<'_> {
    /// Return the NID for a named type, creating a SOURCELESS placeholder stub
    /// when no package-qualified node already exists. Mirrors Go's
    /// `ensure_named_node`.
    ///
    /// The stub carries no `source_file` so the corpus-level rewire can collapse
    /// it onto the real definition; a sourced stub would bake the referencing
    /// file's path (extension and all) into the id and block the rewire — the
    /// phantom-duplicate-node bug (#1500/#1402); the referencing file is recorded
    /// as `origin_file` so same-label cross-file stubs split into distinct ids
    /// (#1462/#1515), matching the generic `ensure_named_node`.
    fn ensure_named_node(&mut self, name: &str) -> String {
        let nid1 = make_id(&[self.pkg_scope, name]);
        if self.seen_ids.contains(&nid1) {
            return nid1;
        }
        let nid2 = make_id1(name);
        if self.seen_ids.insert(nid2.clone()) {
            self.nodes.push(Node {
                id: nid2.clone(),
                label: name.to_string(),
                file_type: "code".to_string(),
                source_file: String::new(),
                source_location: Some(String::new()),
                metadata: None,
                origin_file: Some(self.str_path.to_string()),
            });
        }
        nid2
    }

    /// Push a `references` edge from `src` to `tgt` with the given context.
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

    /// Push a plain `embeds` edge from `src` to `tgt` (Go struct/interface embedding).
    fn push_embeds(&mut self, src: &str, tgt: &str, line: usize) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: "embeds".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    }
}

/// Emit `references` edges for a function/method's parameter and result types.
///
/// Mirrors Python `emit_go_method_refs`: direct param types use the
/// `parameter_type` context, result types use `return_type`, and any generic
/// arguments use `generic_arg`.
pub(super) fn emit_go_method_refs(
    rc: &mut GoRefCtx<'_>,
    func_node: tree_sitter::Node<'_>,
    func_nid: &str,
    line: usize,
) {
    if let Some(params) = func_node.child_by_field_name("parameters") {
        let mut cur = params.walk();
        if cur.goto_first_child() {
            loop {
                let p = cur.node();
                if p.kind() == "parameter_declaration"
                    && let Some(type_node) = p.child_by_field_name("type")
                {
                    let mut refs: Vec<(String, bool)> = Vec::new();
                    go_collect_type_refs(type_node, rc.source, false, &mut refs);
                    for (ref_name, is_generic) in refs {
                        let ctx = if is_generic {
                            "generic_arg"
                        } else {
                            "parameter_type"
                        };
                        let tgt = rc.ensure_named_node(&ref_name);
                        if tgt != func_nid {
                            rc.push_ref(func_nid, &tgt, ctx, line);
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    let Some(result) = func_node.child_by_field_name("result") else {
        return;
    };
    if result.kind() == "parameter_list" {
        let mut cur = result.walk();
        if cur.goto_first_child() {
            loop {
                let p = cur.node();
                if p.kind() == "parameter_declaration" {
                    let type_node = p.child_by_field_name("type").or_else(|| {
                        let mut c = p.walk();
                        if c.goto_first_child() {
                            loop {
                                if c.node().is_named() {
                                    return Some(c.node());
                                }
                                if !c.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                        None
                    });
                    if let Some(tn) = type_node {
                        emit_go_result_refs(rc, tn, func_nid, line);
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    } else {
        emit_go_result_refs(rc, result, func_nid, line);
    }
}

/// Emit `return_type` / `generic_arg` references from a single result type node.
fn emit_go_result_refs(
    rc: &mut GoRefCtx<'_>,
    type_node: tree_sitter::Node<'_>,
    func_nid: &str,
    line: usize,
) {
    let mut refs: Vec<(String, bool)> = Vec::new();
    go_collect_type_refs(type_node, rc.source, false, &mut refs);
    for (ref_name, is_generic) in refs {
        let ctx = if is_generic {
            "generic_arg"
        } else {
            "return_type"
        };
        let tgt = rc.ensure_named_node(&ref_name);
        if tgt != func_nid {
            rc.push_ref(func_nid, &tgt, ctx, line);
        }
    }
}

/// Emit `embeds` / `references[field]` edges for a `type_spec`'s struct fields,
/// and `embeds` / `references[generic_arg]` edges for interface embedding.
///
/// A struct field with no name and a direct (non-generic) type is an embedded
/// field → `embeds`; named fields and generic args become `references`. Mirrors
/// the struct/interface body handling added to Python `extract_go`.
pub(super) fn emit_go_type_body_refs(
    rc: &mut GoRefCtx<'_>,
    type_spec: tree_sitter::Node<'_>,
    type_nid: &str,
) {
    let mut type_body: Option<tree_sitter::Node<'_>> = None;
    let mut cur = type_spec.walk();
    if cur.goto_first_child() {
        loop {
            if matches!(cur.node().kind(), "struct_type" | "interface_type") {
                type_body = Some(cur.node());
                break;
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    let Some(type_body) = type_body else {
        return;
    };

    if type_body.kind() == "struct_type" {
        let mut fdl_cur = type_body.walk();
        if !fdl_cur.goto_first_child() {
            return;
        }
        loop {
            if fdl_cur.node().kind() == "field_declaration_list" {
                let mut fcur = fdl_cur.node().walk();
                if fcur.goto_first_child() {
                    loop {
                        if fcur.node().kind() == "field_declaration" {
                            emit_go_struct_field_refs(rc, fcur.node(), type_nid);
                        }
                        if !fcur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if !fdl_cur.goto_next_sibling() {
                break;
            }
        }
    } else {
        // interface_type — embedded interfaces appear as `type_elem`.
        let mut ecur = type_body.walk();
        if !ecur.goto_first_child() {
            return;
        }
        loop {
            if ecur.node().kind() == "type_elem" {
                let line = ecur.node().start_position().row + 1;
                let mut refs: Vec<(String, bool)> = Vec::new();
                let mut scur = ecur.node().walk();
                if scur.goto_first_child() {
                    loop {
                        if scur.node().is_named() {
                            go_collect_type_refs(scur.node(), rc.source, false, &mut refs);
                        }
                        if !scur.goto_next_sibling() {
                            break;
                        }
                    }
                }
                for (ref_name, is_generic) in refs {
                    let tgt = rc.ensure_named_node(&ref_name);
                    if tgt == type_nid {
                        continue;
                    }
                    if is_generic {
                        rc.push_ref(type_nid, &tgt, "generic_arg", line);
                    } else {
                        rc.push_embeds(type_nid, &tgt, line);
                    }
                }
            }
            if !ecur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Emit edges for a single Go struct `field_declaration`.
fn emit_go_struct_field_refs(rc: &mut GoRefCtx<'_>, field: tree_sitter::Node<'_>, type_nid: &str) {
    let line = field.start_position().row + 1;
    let mut has_name = false;
    let mut fallback_type: Option<tree_sitter::Node<'_>> = None;
    let mut fcur = field.walk();
    if fcur.goto_first_child() {
        loop {
            let fc = fcur.node();
            if fc.kind() == "field_identifier" {
                has_name = true;
            } else if fallback_type.is_none() && fc.is_named() {
                fallback_type = Some(fc);
            }
            if !fcur.goto_next_sibling() {
                break;
            }
        }
    }
    let Some(type_node) = field.child_by_field_name("type").or(fallback_type) else {
        return;
    };
    let mut refs: Vec<(String, bool)> = Vec::new();
    go_collect_type_refs(type_node, rc.source, false, &mut refs);
    for (ref_name, is_generic) in refs {
        let tgt = rc.ensure_named_node(&ref_name);
        if tgt == type_nid {
            continue;
        }
        if !has_name && !is_generic {
            rc.push_embeds(type_nid, &tgt, line);
        } else {
            let ctx = if is_generic { "generic_arg" } else { "field" };
            rc.push_ref(type_nid, &tgt, ctx, line);
        }
    }
}
