//! Go structural AST walk (functions, methods, types, imports).

use super::read_text;
use super::refs::{GoRefCtx, emit_go_method_refs, emit_go_type_body_refs};
use crate::ids::make_id;
use crate::types::{Edge, Node};
use std::collections::HashSet;

/// Recursively walk a Go AST emitting nodes and edges for functions, methods, and type declarations.
///
/// Handles `function_declaration`, `method_declaration`, `type_declaration`, and `import_declaration`
/// nodes. Descends into all child nodes. Mirrors Python `_walk_go`.
/// Shared state threaded through every [`walk_go`] recursion.
pub(super) struct GoWalkCtx<'a> {
    pub(super) str_path: &'a str,
    pub(super) stem: &'a str,
    pub(super) pkg_scope: &'a str,
    pub(super) file_nid: &'a str,
    pub(super) nodes: &'a mut Vec<Node>,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_ids: &'a mut HashSet<String>,
    pub(super) function_bodies: &'a mut Vec<(String, usize, usize)>,
    pub(super) go_imported_pkgs: &'a mut HashSet<String>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Go's AST node kinds
pub(super) fn walk_go(ctx: &mut GoWalkCtx<'_>, node: tree_sitter::Node<'_>, source: &[u8]) {
    let str_path = ctx.str_path;
    let stem = ctx.stem;
    let pkg_scope = ctx.pkg_scope;
    let file_nid = ctx.file_nid;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let function_bodies = &mut *ctx.function_bodies;
    let go_imported_pkgs = &mut *ctx.go_imported_pkgs;
    let t = node.kind();

    match t {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let func_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let func_nid = make_id(&[stem, func_name]);
                if seen_ids.insert(func_nid.clone()) {
                    nodes.push(Node {
                        id: func_nid.clone(),
                        label: format!("{func_name}()"),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                        origin_file: None,
                    });
                }
                edges.push(Edge {
                    external: false,
                    source: file_nid.to_string(),
                    target: func_nid.clone(),
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                let mut rc = GoRefCtx {
                    source,
                    pkg_scope,
                    str_path,
                    nodes: &mut *nodes,
                    edges: &mut *edges,
                    seen_ids: &mut *seen_ids,
                };
                emit_go_method_refs(&mut rc, node, &func_nid, line);
                if let Some(body) = node.child_by_field_name("body") {
                    function_bodies.push((func_nid, body.start_byte(), body.end_byte()));
                }
            }
        }
        "method_declaration" => {
            let receiver = node.child_by_field_name("receiver");
            let mut receiver_type: Option<String> = None;
            if let Some(recv) = receiver {
                let mut cur = recv.walk();
                if cur.goto_first_child() {
                    loop {
                        let param = cur.node();
                        if param.kind() == "parameter_declaration" {
                            if let Some(type_node) = param.child_by_field_name("type") {
                                let raw = read_text(type_node, source)
                                    .trim_start_matches('*')
                                    .trim()
                                    .to_string();
                                receiver_type = Some(raw);
                            }
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if let Some(name_node) = node.child_by_field_name("name") {
                let method_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let method_nid = if let Some(ref rt) = receiver_type {
                    let parent_nid = make_id(&[pkg_scope, rt]);
                    if seen_ids.insert(parent_nid.clone()) {
                        nodes.push(Node {
                            id: parent_nid.clone(),
                            label: rt.clone(),
                            file_type: "code".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            metadata: None,
                            origin_file: None,
                        });
                    }
                    let mnid = make_id(&[&parent_nid, method_name]);
                    if seen_ids.insert(mnid.clone()) {
                        nodes.push(Node {
                            id: mnid.clone(),
                            label: format!(".{method_name}()"),
                            file_type: "code".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            metadata: None,
                            origin_file: None,
                        });
                    }
                    edges.push(Edge {
                        external: false,
                        source: parent_nid,
                        target: mnid.clone(),
                        relation: "method".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                    mnid
                } else {
                    let mnid = make_id(&[stem, method_name]);
                    if seen_ids.insert(mnid.clone()) {
                        nodes.push(Node {
                            id: mnid.clone(),
                            label: format!("{method_name}()"),
                            file_type: "code".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            metadata: None,
                            origin_file: None,
                        });
                    }
                    edges.push(Edge {
                        external: false,
                        source: file_nid.to_string(),
                        target: mnid.clone(),
                        relation: "contains".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                    mnid
                };
                let mut rc = GoRefCtx {
                    source,
                    pkg_scope,
                    str_path,
                    nodes: &mut *nodes,
                    edges: &mut *edges,
                    seen_ids: &mut *seen_ids,
                };
                emit_go_method_refs(&mut rc, node, &method_nid, line);
                if let Some(body) = node.child_by_field_name("body") {
                    function_bodies.push((method_nid, body.start_byte(), body.end_byte()));
                }
            }
        }
        "type_declaration" => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "type_spec"
                        && let Some(name_node) = child.child_by_field_name("name")
                    {
                        let type_name = read_text(name_node, source);
                        let line = child.start_position().row + 1;
                        let type_nid = make_id(&[pkg_scope, type_name]);
                        if seen_ids.insert(type_nid.clone()) {
                            nodes.push(Node {
                                id: type_nid.clone(),
                                label: type_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                metadata: None,
                                origin_file: None,
                            });
                        }
                        edges.push(Edge {
                            external: false,
                            source: file_nid.to_string(),
                            target: type_nid.clone(),
                            relation: "contains".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        // Struct field embeds/references and interface embedding.
                        let mut rc = GoRefCtx {
                            source,
                            pkg_scope,
                            str_path,
                            nodes: &mut *nodes,
                            edges: &mut *edges,
                            seen_ids: &mut *seen_ids,
                        };
                        emit_go_type_body_refs(&mut rc, child, &type_nid);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "import_declaration" => {
            walk_go_imports(
                node,
                source,
                str_path,
                file_nid,
                edges,
                seen_ids,
                go_imported_pkgs,
            );
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_go(ctx, cur.node(), source);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Walk an `import_declaration` subtree, delegating each `import_spec` to `emit_go_import_spec`.
fn walk_go_imports(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    file_nid: &str,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    go_imported_pkgs: &mut HashSet<String>,
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        match child.kind() {
            "import_spec_list" => {
                let mut c2 = child.walk();
                if c2.goto_first_child() {
                    loop {
                        let spec = c2.node();
                        if spec.kind() == "import_spec" {
                            emit_go_import_spec(
                                spec,
                                source,
                                str_path,
                                file_nid,
                                edges,
                                seen_ids,
                                go_imported_pkgs,
                            );
                        }
                        if !c2.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            "import_spec" => {
                emit_go_import_spec(
                    child,
                    source,
                    str_path,
                    file_nid,
                    edges,
                    seen_ids,
                    go_imported_pkgs,
                );
            }
            _ => {}
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

/// Emit a single `imports_from` edge for one Go `import_spec` node.
///
/// The target NID is derived from the import path string (e.g. `"fmt"` → `go::pkg::fmt`).
/// The package name is also recorded in `go_imported_pkgs` for use during call resolution.
fn emit_go_import_spec(
    spec: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    file_nid: &str,
    edges: &mut Vec<Edge>,
    _seen_ids: &mut HashSet<String>,
    go_imported_pkgs: &mut HashSet<String>,
) {
    if let Some(path_node) = spec.child_by_field_name("path") {
        let raw = read_text(path_node, source).trim_matches('"');
        let tgt_nid = make_id(&["go", "pkg", raw]);
        let line = spec.start_position().row + 1;
        edges.push(Edge {
            external: false,
            source: file_nid.to_string(),
            target: tgt_nid,
            relation: "imports_from".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: Some("import".to_string()),
            confidence_score: None,
        });
        // Track local name (alias or last path segment)
        let alias = spec.child_by_field_name("name");
        let local_name = if let Some(a) = alias {
            read_text(a, source).to_string()
        } else {
            raw.split('/').next_back().unwrap_or("").to_string()
        };
        if !local_name.is_empty() && local_name != "_" && local_name != "." {
            go_imported_pkgs.insert(local_name);
        }
    }
}
