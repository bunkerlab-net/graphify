//! Julia extractor — custom walk over tree-sitter-julia AST.

use std::collections::HashSet;
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract modules, structs, functions, imports, and calls from a `.jl` file.
#[must_use]
pub fn extract_julia(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_julia::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set julia language".to_string()),
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
    // (func_nid, node_start_byte, node_end_byte, is_function_def)
    let mut function_bodies: Vec<(String, usize, usize, bool)> = Vec::new();

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
    walk_julia(
        root,
        &source,
        &str_path,
        &stem,
        &file_nid,
        &file_nid,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut function_bodies,
    );

    // Second pass: call edges
    for (func_nid, node_start, node_end, is_func_def) in &function_bodies {
        let tree_root = tree.root_node();
        if *is_func_def {
            // Walk children, skip signature child
            walk_calls_julia_children(
                tree_root,
                &source,
                &str_path,
                &stem,
                func_nid,
                *node_start,
                *node_end,
                &mut edges,
                &seen_ids,
            );
        } else {
            walk_calls_julia(
                tree_root,
                &source,
                &str_path,
                &stem,
                func_nid,
                *node_start,
                *node_end,
                &mut edges,
                &seen_ids,
            );
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

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
                let callee = child.walk().goto_first_child().then(|| {
                    let mut c = child.walk();
                    c.goto_first_child();
                    c.node()
                });
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
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_julia(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    file_nid: &str,
    scope_nid: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize, bool)>,
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
                let mod_nid = make_id(&[stem, mod_name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(mod_nid.clone()) {
                    nodes.push(Node {
                        id: mod_nid.clone(),
                        label: mod_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: mod_nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_julia(
                            cur.node(),
                            source,
                            str_path,
                            stem,
                            file_nid,
                            &mod_nid,
                            nodes,
                            edges,
                            seen_ids,
                            function_bodies,
                        );
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
                        let struct_nid = make_id(&[stem, struct_name]);
                        if seen_ids.insert(struct_nid.clone()) {
                            nodes.push(Node {
                                id: struct_nid.clone(),
                                label: struct_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                            });
                        }
                        edges.push(Edge {
                            source: scope_nid.to_string(),
                            target: struct_nid.clone(),
                            relation: "defines".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        if identifiers.len() >= 2 {
                            let super_name = read_text(identifiers[identifiers.len() - 1], source);
                            let super_nid = make_id(&[stem, super_name]);
                            edges.push(Edge {
                                source: struct_nid,
                                target: super_nid,
                                relation: "inherits".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: None,
                                confidence_score: None,
                            });
                        }
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
                        let struct_nid = make_id(&[stem, struct_name]);
                        if seen_ids.insert(struct_nid.clone()) {
                            nodes.push(Node {
                                id: struct_nid.clone(),
                                label: struct_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                            });
                        }
                        edges.push(Edge {
                            source: scope_nid.to_string(),
                            target: struct_nid,
                            relation: "defines".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
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
                    let abs_nid = make_id(&[stem, abs_name]);
                    let line = node.start_position().row + 1;
                    if seen_ids.insert(abs_nid.clone()) {
                        nodes.push(Node {
                            id: abs_nid.clone(),
                            label: abs_name.to_string(),
                            file_type: "code".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                        });
                    }
                    edges.push(Edge {
                        source: scope_nid.to_string(),
                        target: abs_nid,
                        relation: "defines".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
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
                let func_nid = make_id(&[stem, &func_name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(func_nid.clone()) {
                    nodes.push(Node {
                        id: func_nid.clone(),
                        label: format!("{func_name}()"),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: scope_nid.to_string(),
                    target: func_nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                function_bodies.push((func_nid, node.start_byte(), node.end_byte(), true));
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
                    let func_nid = make_id(&[stem, func_name]);
                    let line = node.start_position().row + 1;
                    if seen_ids.insert(func_nid.clone()) {
                        nodes.push(Node {
                            id: func_nid.clone(),
                            label: format!("{func_name}()"),
                            file_type: "code".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                        });
                    }
                    edges.push(Edge {
                        source: scope_nid.to_string(),
                        target: func_nid.clone(),
                        relation: "defines".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                    // Walk RHS only (last child)
                    if node.child_count() >= 3
                        && let Some(rhs) = node.child(node.child_count() - 1)
                    {
                        function_bodies.push((func_nid, rhs.start_byte(), rhs.end_byte(), false));
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
                        seen_ids.insert(imp_nid.clone());
                        nodes.push(Node {
                            id: imp_nid.clone(),
                            label: mod_name.to_string(),
                            file_type: "code".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                        });
                        edges.push(Edge {
                            source: scope_nid.to_string(),
                            target: imp_nid,
                            relation: "imports".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
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
                            seen_ids.insert(pkg_nid.clone());
                            nodes.push(Node {
                                id: pkg_nid.clone(),
                                label: pkg_name.to_string(),
                                file_type: "code".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                            });
                            edges.push(Edge {
                                source: scope_nid.to_string(),
                                target: pkg_nid,
                                relation: "imports".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: str_path.to_string(),
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
                    walk_julia(
                        cur.node(),
                        source,
                        str_path,
                        stem,
                        file_nid,
                        scope_nid,
                        nodes,
                        edges,
                        seen_ids,
                        function_bodies,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` edges within a Julia function body's byte range.
///
/// Skips nested `function_definition` nodes. Emits `calls` edges for `call_expression` nodes
/// whose callee matches a known NID. Mirrors Python `_walk_calls_julia`.
#[allow(clippy::too_many_arguments)]
fn walk_calls_julia(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    func_nid: &str,
    body_start: usize,
    body_end: usize,
    edges: &mut Vec<Edge>,
    seen_ids: &HashSet<String>,
) {
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if matches!(
        node.kind(),
        "function_definition" | "short_function_definition"
    ) {
        return;
    }
    if node.kind() == "call_expression" && node.child_count() > 0 {
        let callee = {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                Some(cur.node())
            } else {
                None
            }
        };
        if let Some(callee_node) = callee {
            if callee_node.kind() == "identifier" {
                let callee_name = read_text(callee_node, source);
                let target_nid = make_id(&[stem, callee_name]);
                if seen_ids.contains(&target_nid) && target_nid != func_nid {
                    edges.push(Edge {
                        source: func_nid.to_string(),
                        target: target_nid,
                        relation: "calls".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{}", node.start_position().row + 1)),
                        weight: 1.0,
                        context: Some("call".to_string()),
                        confidence_score: None,
                    });
                }
            } else if callee_node.kind() == "field_expression" && callee_node.child_count() >= 3 {
                let method_node = callee_node.child(callee_node.child_count() - 1);
                if let Some(mn) = method_node {
                    let method_name = read_text(mn, source);
                    let target_nid = make_id(&[stem, method_name]);
                    if seen_ids.contains(&target_nid) && target_nid != func_nid {
                        edges.push(Edge {
                            source: func_nid.to_string(),
                            target: target_nid,
                            relation: "calls".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{}", node.start_position().row + 1)),
                            weight: 1.0,
                            context: Some("call".to_string()),
                            confidence_score: None,
                        });
                    }
                }
            }
        }
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_julia(
                cur.node(),
                source,
                str_path,
                stem,
                func_nid,
                body_start,
                body_end,
                edges,
                seen_ids,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Walk the body children of a `function_definition` node, calling `walk_calls_julia` on each.
///
/// Finds the `function_definition` node by byte range, then iterates its children starting
/// after the signature, so nested function bodies are attributed to the right caller.
// Walk children of a function_definition node (skipping signature)
#[allow(clippy::too_many_arguments)]
fn walk_calls_julia_children(
    tree_root: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    func_nid: &str,
    node_start: usize,
    node_end: usize,
    edges: &mut Vec<Edge>,
    seen_ids: &HashSet<String>,
) {
    // Find the function_definition node by byte range
    fn find_node(
        n: tree_sitter::Node<'_>,
        start: usize,
        end: usize,
    ) -> Option<tree_sitter::Node<'_>> {
        if n.start_byte() == start && n.end_byte() == end && n.kind() == "function_definition" {
            return Some(n);
        }
        let mut cur = n.walk();
        if cur.goto_first_child() {
            loop {
                if let Some(found) = find_node(cur.node(), start, end) {
                    return Some(found);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    let Some(func_node) = find_node(tree_root, node_start, node_end) else {
        return;
    };
    let mut cur = func_node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() != "signature" {
                walk_calls_julia(
                    child, source, str_path, stem, func_nid, node_start, node_end, edges, seen_ids,
                );
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
