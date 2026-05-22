//! Objective-C extractor — custom walk over tree-sitter-objc AST.

use std::collections::HashSet;
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract interfaces, implementations, protocols, methods, and imports from `.m`/`.mm`/`.h` files.
#[must_use]
pub fn extract_objc(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_objc::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set objc language".to_string()),
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
    let mut method_bodies: Vec<(String, usize, usize)> = Vec::new();

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
    walk_objc(
        root,
        &source,
        &str_path,
        &stem,
        &file_nid,
        None,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut method_bodies,
    );

    // Second pass: calls inside method bodies
    let all_method_nids: HashSet<String> = nodes
        .iter()
        .filter(|n| n.id != file_nid)
        .map(|n| n.id.clone())
        .collect();
    let mut seen_calls: HashSet<(String, String)> = HashSet::new();

    for (caller_nid, body_start, body_end) in &method_bodies {
        walk_calls_objc(
            tree.root_node(),
            &source,
            &str_path,
            caller_nid,
            *body_start,
            *body_end,
            &all_method_nids,
            &mut edges,
            &mut seen_calls,
        );
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Recursively walk an `ObjC` AST emitting nodes for interfaces, implementations, and methods.
///
/// Handles `@interface`, `@implementation`, `@protocol`, instance/class method declarations
/// and definitions, and `#import` / `@import` directives. Mirrors Python `_walk_objc`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_objc(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    file_nid: &str,
    parent_nid: Option<&str>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    method_bodies: &mut Vec<(String, usize, usize)>,
) {
    let t = node.kind();
    let line = node.start_position().row + 1;

    match t {
        "preproc_include" => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "system_lib_string" {
                        let raw = read_text(child, source).trim_matches(|c| c == '<' || c == '>');
                        let module = raw.split('/').next_back().unwrap_or("").replace(".h", "");
                        if !module.is_empty() {
                            let tgt_nid = make_id1(&module);
                            edges.push(Edge {
                                source: file_nid.to_string(),
                                target: tgt_nid,
                                relation: "imports".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: Some("import".to_string()),
                                confidence_score: None,
                            });
                        }
                    } else if child.kind() == "string_literal" {
                        let mut sc = child.walk();
                        if sc.goto_first_child() {
                            loop {
                                if sc.node().kind() == "string_content" {
                                    let raw = read_text(sc.node(), source);
                                    let module =
                                        raw.split('/').next_back().unwrap_or("").replace(".h", "");
                                    if !module.is_empty() {
                                        let tgt_nid = make_id1(&module);
                                        edges.push(Edge {
                                            source: file_nid.to_string(),
                                            target: tgt_nid,
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
                                if !sc.goto_next_sibling() {
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
        "class_interface" => {
            let identifiers: Vec<tree_sitter::Node<'_>> = {
                let mut ids = vec![];
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "identifier" {
                            ids.push(cur.node());
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
                ids
            };
            if identifiers.is_empty() {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_objc(
                            cur.node(),
                            source,
                            str_path,
                            stem,
                            file_nid,
                            parent_nid,
                            nodes,
                            edges,
                            seen_ids,
                            method_bodies,
                        );
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
                return;
            }
            let name = read_text(identifiers[0], source);
            let cls_nid = make_id(&[stem, name]);
            if seen_ids.insert(cls_nid.clone()) {
                nodes.push(Node {
                    id: cls_nid.clone(),
                    label: name.to_string(),
                    file_type: "code".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                });
            }
            edges.push(Edge {
                source: file_nid.to_string(),
                target: cls_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            // Superclass and protocol adoption
            let mut colon_seen = false;
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == ":" {
                        colon_seen = true;
                    } else if colon_seen && child.kind() == "identifier" {
                        let super_nid = make_id1(read_text(child, source));
                        edges.push(Edge {
                            source: cls_nid.clone(),
                            target: super_nid,
                            relation: "inherits".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        colon_seen = false;
                    } else if child.kind() == "parameterized_arguments" {
                        let mut pc = child.walk();
                        if pc.goto_first_child() {
                            loop {
                                if pc.node().kind() == "type_name" {
                                    let mut tc = pc.node().walk();
                                    if tc.goto_first_child() {
                                        loop {
                                            if tc.node().kind() == "type_identifier" {
                                                let proto_nid =
                                                    make_id1(read_text(tc.node(), source));
                                                edges.push(Edge {
                                                    source: cls_nid.clone(),
                                                    target: proto_nid,
                                                    relation: "imports".to_string(),
                                                    confidence: "EXTRACTED".to_string(),
                                                    source_file: str_path.to_string(),
                                                    source_location: Some(format!("L{line}")),
                                                    weight: 1.0,
                                                    context: Some("import".to_string()),
                                                    confidence_score: None,
                                                });
                                            }
                                            if !tc.goto_next_sibling() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                if !pc.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    } else if child.kind() == "method_declaration" {
                        walk_objc(
                            child,
                            source,
                            str_path,
                            stem,
                            file_nid,
                            Some(&cls_nid),
                            nodes,
                            edges,
                            seen_ids,
                            method_bodies,
                        );
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "class_implementation" => {
            let name = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "identifier" {
                            f = Some(read_text(cur.node(), source).to_string());
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
            if let Some(n) = name {
                let impl_nid = make_id(&[stem, &n]);
                if seen_ids.insert(impl_nid.clone()) {
                    nodes.push(Node {
                        id: impl_nid.clone(),
                        label: n,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                    edges.push(Edge {
                        source: file_nid.to_string(),
                        target: impl_nid.clone(),
                        relation: "contains".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                }
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "implementation_definition" {
                            let mut c2 = cur.node().walk();
                            if c2.goto_first_child() {
                                loop {
                                    walk_objc(
                                        c2.node(),
                                        source,
                                        str_path,
                                        stem,
                                        file_nid,
                                        Some(&impl_nid),
                                        nodes,
                                        edges,
                                        seen_ids,
                                        method_bodies,
                                    );
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
            }
        }
        "protocol_declaration" => {
            let name = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "identifier" {
                            f = Some(read_text(cur.node(), source).to_string());
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
            if let Some(n) = name {
                let proto_nid = make_id(&[stem, &n]);
                if seen_ids.insert(proto_nid.clone()) {
                    nodes.push(Node {
                        id: proto_nid.clone(),
                        label: format!("<{n}>"),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: proto_nid.clone(),
                    relation: "contains".to_string(),
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
                        walk_objc(
                            cur.node(),
                            source,
                            str_path,
                            stem,
                            file_nid,
                            Some(&proto_nid),
                            nodes,
                            edges,
                            seen_ids,
                            method_bodies,
                        );
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "method_declaration" | "method_definition" => {
            let container = parent_nid.unwrap_or(file_nid);
            let mut parts: Vec<&str> = Vec::new();
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "identifier" {
                        parts.push(read_text(cur.node(), source));
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            if let Some(method_name) = parts.first().copied() {
                let method_nid = make_id(&[container, method_name]);
                if seen_ids.insert(method_nid.clone()) {
                    nodes.push(Node {
                        id: method_nid.clone(),
                        label: format!("-{method_name}"),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: container.to_string(),
                    target: method_nid.clone(),
                    relation: "method".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                if t == "method_definition" {
                    method_bodies.push((method_nid, node.start_byte(), node.end_byte()));
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_objc(
                        cur.node(),
                        source,
                        str_path,
                        stem,
                        file_nid,
                        parent_nid,
                        nodes,
                        edges,
                        seen_ids,
                        method_bodies,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` edges within an `ObjC` method body.
///
/// Recurses through the body AST, emitting `calls` edges for `message_expression` nodes whose
/// selector matches a known method NID. Mirrors Python `_walk_calls_objc`.
#[allow(clippy::too_many_arguments)]
fn walk_calls_objc(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
    all_method_nids: &HashSet<String>,
    edges: &mut Vec<Edge>,
    seen_calls: &mut HashSet<(String, String)>,
) {
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if node.kind() == "message_expression" {
        let mut sel: Vec<&str> = Vec::new();
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "selector" {
                    sel.push(read_text(child, source));
                } else if child.kind() == "keyword_argument_list" {
                    let mut kc = child.walk();
                    if kc.goto_first_child() {
                        loop {
                            if kc.node().kind() == "keyword_argument" {
                                let mut sc = kc.node().walk();
                                if sc.goto_first_child() {
                                    loop {
                                        if sc.node().kind() == "selector" {
                                            sel.push(read_text(sc.node(), source));
                                        }
                                        if !sc.goto_next_sibling() {
                                            break;
                                        }
                                    }
                                }
                            }
                            if !kc.goto_next_sibling() {
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
        let method_name = sel.join("");
        if !method_name.is_empty() {
            // Match against all method nids by suffix
            let suffix_key = make_id1(&method_name);
            for candidate in all_method_nids {
                if candidate.ends_with(suffix_key.trim_start_matches('_')) {
                    let pair = (caller_nid.to_string(), candidate.clone());
                    if !seen_calls.contains(&pair) && caller_nid != candidate {
                        seen_calls.insert(pair);
                        let line = node.start_position().row + 1;
                        edges.push(Edge {
                            source: caller_nid.to_string(),
                            target: candidate.clone(),
                            relation: "calls".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
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
            walk_calls_objc(
                cur.node(),
                source,
                str_path,
                caller_nid,
                body_start,
                body_end,
                all_method_nids,
                edges,
                seen_calls,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
