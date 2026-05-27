//! Zig extractor — custom walk over tree-sitter-zig AST.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, structs, enums, unions, and imports from a `.zig` file.
#[must_use]
// Single-pass tree-sitter extractor: node/edge emission shares accumulator
// state across function/struct/enum/import branches, so splitting into helpers
// would separate related logic.
#[allow(clippy::too_many_lines)]
pub fn extract_zig(path: &Path) -> FileResult {
    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return FileResult {
                nodes: vec![],
                edges: vec![],
                raw_calls: vec![],
                error: Some(e.to_string()),
            };
        }
    };

    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_zig::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set zig language".to_string()),
        };
    }
    let Some(tree) = parser.parse(&source, None) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("parse failed".to_string()),
        };
    };

    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();

    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut function_bodies: Vec<(String, usize, usize)> = Vec::new();

    let file_nid = make_id1(&str_path);
    seen_ids.insert(file_nid.clone());
    nodes.push(Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
    });

    let root = tree.root_node();
    {
        let mut walk_ctx = ZigWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
        };
        walk_zig(&mut walk_ctx, root, &source, None);
    }

    // Build label→nid map
    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    {
        let mut call_ctx = ZigCallCtx {
            str_path: &str_path,
            label_to_nid: &label_to_nid,
            edges: &mut edges,
            seen_call_pairs: &mut seen_call_pairs,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_start, body_end) in &function_bodies {
            walk_calls_zig(
                &mut call_ctx,
                tree.root_node(),
                &source,
                caller_nid,
                *body_start,
                *body_end,
            );
        }
    }

    let clean_edges: Vec<Edge> = edges
        .into_iter()
        .filter(|e| {
            seen_ids.contains(&e.source)
                && (seen_ids.contains(&e.target) || e.relation == "imports_from")
        })
        .collect();

    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}

/// Recursively walk a Zig AST emitting nodes for functions, structs, enums, and unions.
///
/// Handles `function_declaration`, `struct_declaration`, `enum_declaration`, and
/// `@import` builtin calls. Mirrors Python `_walk_zig`.
/// Shared state threaded through every [`walk_zig`] recursion.
struct ZigWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    function_bodies: &'a mut Vec<(String, usize, usize)>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over many AST node kinds
fn walk_zig(
    ctx: &mut ZigWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    parent_struct_nid: Option<&str>,
) {
    let str_path = ctx.str_path;
    let stem = ctx.stem;
    let file_nid = ctx.file_nid;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let function_bodies = &mut *ctx.function_bodies;

    let t = node.kind();

    match t {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let func_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let (func_nid, label, parent, relation) = if let Some(pnid) = parent_struct_nid {
                    (
                        make_id(&[pnid, func_name]),
                        format!(".{func_name}()"),
                        pnid.to_string(),
                        "method",
                    )
                } else {
                    (
                        make_id(&[stem, func_name]),
                        format!("{func_name}()"),
                        file_nid.to_string(),
                        "contains",
                    )
                };
                if seen_ids.insert(func_nid.clone()) {
                    nodes.push(Node {
                        id: func_nid.clone(),
                        label,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    source: parent,
                    target: func_nid.clone(),
                    relation: relation.to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                if let Some(body) = node.child_by_field_name("body") {
                    function_bodies.push((func_nid, body.start_byte(), body.end_byte()));
                }
            }
        }
        "variable_declaration" => {
            // Find name (identifier) and value (struct/enum/union/builtin_function/field_expression)
            let mut name_node: Option<tree_sitter::Node<'_>> = None;
            let mut value_node: Option<tree_sitter::Node<'_>> = None;
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    match child.kind() {
                        "identifier" if name_node.is_none() => {
                            name_node = Some(child);
                        }
                        "struct_declaration" | "enum_declaration" | "union_declaration"
                        | "builtin_function" | "field_expression" => {
                            value_node = Some(child);
                        }
                        _ => {}
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            if let (Some(vn), Some(nn)) = (value_node, name_node) {
                match vn.kind() {
                    "struct_declaration" => {
                        let struct_name = read_text(nn, source);
                        let line = node.start_position().row + 1;
                        let struct_nid = make_id(&[stem, struct_name]);
                        if seen_ids.insert(struct_nid.clone()) {
                            nodes.push(Node {
                                id: struct_nid.clone(),
                                label: struct_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                metadata: None,
                            });
                        }
                        edges.push(Edge {
                            source: file_nid.to_string(),
                            target: struct_nid.clone(),
                            relation: "contains".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        let mut c2 = vn.walk();
                        if c2.goto_first_child() {
                            loop {
                                walk_zig(ctx, c2.node(), source, Some(&struct_nid));
                                if !c2.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    }
                    "enum_declaration" | "union_declaration" => {
                        let type_name = read_text(nn, source);
                        let line = node.start_position().row + 1;
                        let type_nid = make_id(&[stem, type_name]);
                        if seen_ids.insert(type_nid.clone()) {
                            nodes.push(Node {
                                id: type_nid.clone(),
                                label: type_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                metadata: None,
                            });
                        }
                        edges.push(Edge {
                            source: file_nid.to_string(),
                            target: type_nid,
                            relation: "contains".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                    }
                    "builtin_function" | "field_expression" => {
                        // Check for @import
                        extract_zig_import(node, source, str_path, file_nid, edges);
                    }
                    _ => {}
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_zig(ctx, cur.node(), source, parent_struct_nid);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Emit an `imports_from` edge for a Zig `@import("...")` builtin call.
///
/// Only processes string-literal arguments; template or non-string arguments are silently skipped.
fn extract_zig_import(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    file_nid: &str,
    edges: &mut Vec<Edge>,
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "builtin_function" {
            // Look for @import or @cImport with a string argument
            let mut bi: Option<&str> = None;
            let mut args_node: Option<tree_sitter::Node<'_>> = None;
            let mut c2 = child.walk();
            if c2.goto_first_child() {
                loop {
                    let sub = c2.node();
                    match sub.kind() {
                        "builtin_identifier" => {
                            bi = Some(
                                std::str::from_utf8(&source[sub.start_byte()..sub.end_byte()])
                                    .unwrap_or(""),
                            );
                        }
                        "arguments" => {
                            args_node = Some(sub);
                        }
                        _ => {}
                    }
                    if !c2.goto_next_sibling() {
                        break;
                    }
                }
            }
            if matches!(bi, Some("@import" | "@cImport"))
                && let Some(args) = args_node
            {
                let mut a = args.walk();
                if a.goto_first_child() {
                    loop {
                        let arg = a.node();
                        if matches!(arg.kind(), "string_literal" | "string") {
                            let raw = read_text(arg, source).trim_matches('"');
                            let module_name = raw
                                .split('/')
                                .next_back()
                                .unwrap_or("")
                                .split('.')
                                .next()
                                .unwrap_or("");
                            if !module_name.is_empty() {
                                let tgt_nid = make_id1(module_name);
                                let line = node.start_position().row + 1;
                                edges.push(Edge {
                                    source: file_nid.to_string(),
                                    target: tgt_nid,
                                    relation: "imports_from".to_string(),
                                    confidence: "EXTRACTED".to_string(),
                                    source_file: str_path.to_string(),
                                    source_location: Some(format!("L{line}")),
                                    weight: 1.0,
                                    context: None,
                                    confidence_score: None,
                                });
                            }
                            return;
                        }
                        if !a.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        } else if child.kind() == "field_expression" {
            extract_zig_import(child, source, str_path, file_nid, edges);
            return;
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

/// Collect `calls` edges within a Zig function body's byte range.
///
/// Recurses through the body AST, emitting `calls` edges for `call_expression` nodes whose
/// callee matches a known NID. Mirrors Python `_walk_calls_zig`.
/// Shared state threaded through every [`walk_calls_zig`] recursion.
struct ZigCallCtx<'a> {
    str_path: &'a str,
    label_to_nid: &'a HashMap<String, String>,
    edges: &'a mut Vec<Edge>,
    seen_call_pairs: &'a mut HashSet<(String, String)>,
    raw_calls: &'a mut Vec<RawCall>,
}

fn walk_calls_zig(
    ctx: &mut ZigCallCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
) {
    let str_path = ctx.str_path;
    let label_to_nid = ctx.label_to_nid;
    let edges = &mut *ctx.edges;
    let seen_call_pairs = &mut *ctx.seen_call_pairs;
    let raw_calls = &mut *ctx.raw_calls;

    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if node.kind() == "function_declaration" {
        return;
    }

    if node.kind() == "call_expression"
        && let Some(func_node) = node.child_by_field_name("function")
    {
        let fn_text = read_text(func_node, source);
        let callee = fn_text.split('.').next_back().unwrap_or("").to_string();
        let is_member_call = fn_text.contains('.');
        // Find matching node label. The dotted fallback originally tried
        // a different key but ended up probing the same one after the
        // trim, so a single lookup suffices.
        let tgt_nid = label_to_nid.get(&callee.to_lowercase()).cloned();
        if let Some(tgt) = tgt_nid {
            if tgt != caller_nid {
                let pair = (caller_nid.to_string(), tgt.clone());
                if seen_call_pairs.insert(pair) {
                    let line = node.start_position().row + 1;
                    edges.push(Edge {
                        source: caller_nid.to_string(),
                        target: tgt,
                        relation: "calls".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                }
            }
        } else if !callee.is_empty() {
            raw_calls.push(RawCall {
                caller_nid: caller_nid.to_string(),
                callee,
                is_member_call,
                source_file: str_path.to_string(),
                source_location: format!("L{}", node.start_position().row + 1),
            });
        }
    }

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_zig(ctx, cur.node(), source, caller_nid, body_start, body_end);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
