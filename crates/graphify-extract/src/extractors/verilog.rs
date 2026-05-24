//! Verilog/SystemVerilog extractor — custom walk over tree-sitter-verilog AST.

use std::collections::HashSet;
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract modules, functions, tasks, package imports, and instantiations from `.v`/`.sv` files.
#[must_use]
pub fn extract_verilog(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_verilog::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set verilog language".to_string()),
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
    });

    let root = tree.root_node();
    {
        let mut walk_ctx = VerilogWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
        };
        walk_verilog(&mut walk_ctx, root, &source, None);
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Recursively walk a Verilog/SystemVerilog AST emitting nodes for modules, functions, and tasks.
///
/// Handles `module_declaration`, `function_declaration`, `task_declaration`, and
/// `module_instantiation` (as `uses` edges). Mirrors Python `_walk_verilog`.
/// Shared state threaded through every [`walk_verilog`] recursion.
struct VerilogWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Verilog's AST node kinds
fn walk_verilog(
    ctx: &mut VerilogWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    module_nid: Option<&str>,
) {
    let t = node.kind();
    match t {
        "module_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let mod_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let nid = make_id(&[ctx.stem, mod_name]);
                if ctx.seen_ids.insert(nid.clone()) {
                    ctx.nodes.push(Node {
                        id: nid.clone(),
                        label: mod_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                ctx.edges.push(Edge {
                    source: ctx.file_nid.to_string(),
                    target: nid.clone(),
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
                        walk_verilog(ctx, cur.node(), source, Some(&nid));
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "function_declaration" | "function_prototype" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let func_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let parent = module_nid.unwrap_or(ctx.file_nid);
                let nid = make_id(&[parent, func_name]);
                if ctx.seen_ids.insert(nid.clone()) {
                    ctx.nodes.push(Node {
                        id: nid.clone(),
                        label: format!("{func_name}()"),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                ctx.edges.push(Edge {
                    source: parent.to_string(),
                    target: nid,
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
            }
        }
        "task_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let task_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let parent = module_nid.unwrap_or(ctx.file_nid);
                let nid = make_id(&[parent, task_name]);
                if ctx.seen_ids.insert(nid.clone()) {
                    ctx.nodes.push(Node {
                        id: nid.clone(),
                        label: task_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                ctx.edges.push(Edge {
                    source: parent.to_string(),
                    target: nid,
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
            }
        }
        "package_import_declaration" => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "package_import_item" {
                        let pkg_text = read_text(cur.node(), source);
                        let pkg_name = pkg_text.split("::").next().unwrap_or("").trim().to_string();
                        if !pkg_name.is_empty() {
                            let line = node.start_position().row + 1;
                            let tgt_nid = make_id1(&pkg_name);
                            if ctx.seen_ids.insert(tgt_nid.clone()) {
                                ctx.nodes.push(Node {
                                    id: tgt_nid.clone(),
                                    label: pkg_name,
                                    file_type: "code".to_string(),
                                    source_file: ctx.str_path.to_string(),
                                    source_location: Some(format!("L{line}")),
                                });
                            }
                            let src = module_nid.unwrap_or(ctx.file_nid);
                            ctx.edges.push(Edge {
                                source: src.to_string(),
                                target: tgt_nid,
                                relation: "imports_from".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: ctx.str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: None,
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
        "module_instantiation" => {
            if let Some(type_node) = node.child_by_field_name("module_type")
                && let Some(mnid) = module_nid
            {
                let inst_type = read_text(type_node, source).trim().to_string();
                if !inst_type.is_empty() {
                    let line = node.start_position().row + 1;
                    let tgt_nid = make_id1(&inst_type);
                    if ctx.seen_ids.insert(tgt_nid.clone()) {
                        ctx.nodes.push(Node {
                            id: tgt_nid.clone(),
                            label: inst_type,
                            file_type: "code".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                        });
                    }
                    ctx.edges.push(Edge {
                        source: mnid.to_string(),
                        target: tgt_nid,
                        relation: "instantiates".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                }
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_verilog(ctx, cur.node(), source, module_nid);
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
                    walk_verilog(ctx, cur.node(), source, module_nid);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}
