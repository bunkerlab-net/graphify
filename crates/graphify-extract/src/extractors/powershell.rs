//! PowerShell extractor — custom walk over tree-sitter-powershell AST.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};

static PS_SKIP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "using", "return", "if", "else", "elseif", "foreach", "for", "while", "do", "switch",
        "try", "catch", "finally", "throw", "break", "continue", "exit", "param", "begin",
        "process", "end",
    ]
    .into_iter()
    .collect()
});

fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, classes, methods, and using statements from a `.ps1` file.
#[must_use]
pub fn extract_powershell(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_powershell::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set powershell language".to_string()),
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
    });

    let root = tree.root_node();
    walk_ps(
        root,
        &source,
        &str_path,
        &stem,
        &file_nid,
        None,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut function_bodies,
    );

    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();

    for (caller_nid, body_start, body_end) in &function_bodies {
        walk_calls_ps(
            tree.root_node(),
            &source,
            &str_path,
            caller_nid,
            *body_start,
            *body_end,
            &label_to_nid,
            &mut edges,
            &mut seen_call_pairs,
            &mut raw_calls,
        );
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

fn find_script_block_body(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return None;
    }
    loop {
        let child = cur.node();
        if child.kind() == "script_block" {
            let mut c2 = child.walk();
            if c2.goto_first_child() {
                loop {
                    if c2.node().kind() == "script_block_body" {
                        return Some(c2.node());
                    }
                    if !c2.goto_next_sibling() {
                        break;
                    }
                }
            }
            return Some(child);
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    None
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_ps(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    file_nid: &str,
    parent_class_nid: Option<&str>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
) {
    let t = node.kind();

    match t {
        "function_statement" => {
            let name_node = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut found = None;
                    loop {
                        if cur.node().kind() == "function_name" {
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
                let func_name = read_text(nn, source);
                let line = node.start_position().row + 1;
                let func_nid = make_id(&[stem, func_name]);
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
                if let Some(body) = find_script_block_body(node) {
                    function_bodies.push((func_nid, body.start_byte(), body.end_byte()));
                }
            }
        }
        "class_statement" => {
            let name_node = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut found = None;
                    loop {
                        if cur.node().kind() == "simple_name" {
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
                let class_name = read_text(nn, source);
                let line = node.start_position().row + 1;
                let class_nid = make_id(&[stem, class_name]);
                if seen_ids.insert(class_nid.clone()) {
                    nodes.push(Node {
                        id: class_nid.clone(),
                        label: class_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: class_nid.clone(),
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
                        walk_ps(
                            cur.node(),
                            source,
                            str_path,
                            stem,
                            file_nid,
                            Some(&class_nid),
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
        "class_method_definition" => {
            let name_node = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut found = None;
                    loop {
                        if cur.node().kind() == "simple_name" {
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
                let method_name = read_text(nn, source);
                let line = node.start_position().row + 1;
                let (method_nid, label, parent, relation) = if let Some(cnid) = parent_class_nid {
                    (
                        make_id(&[cnid, method_name]),
                        format!(".{method_name}()"),
                        cnid.to_string(),
                        "method",
                    )
                } else {
                    (
                        make_id(&[stem, method_name]),
                        format!("{method_name}()"),
                        file_nid.to_string(),
                        "contains",
                    )
                };
                if seen_ids.insert(method_nid.clone()) {
                    nodes.push(Node {
                        id: method_nid.clone(),
                        label,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: parent,
                    target: method_nid.clone(),
                    relation: relation.to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                if let Some(body) = find_script_block_body(node) {
                    function_bodies.push((method_nid, body.start_byte(), body.end_byte()));
                }
            }
        }
        "command" => {
            let cmd_name_node = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut found = None;
                    loop {
                        if cur.node().kind() == "command_name" {
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
            if let Some(cmd_nn) = cmd_name_node {
                let cmd_text = read_text(cmd_nn, source).to_lowercase();
                if cmd_text == "using" {
                    let mut tokens: Vec<String> = Vec::new();
                    let mut cur = node.walk();
                    if cur.goto_first_child() {
                        loop {
                            if cur.node().kind() == "command_elements" {
                                let mut c2 = cur.node().walk();
                                if c2.goto_first_child() {
                                    loop {
                                        if c2.node().kind() == "generic_token" {
                                            tokens.push(read_text(c2.node(), source).to_string());
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
                    let module_tokens: Vec<&str> = tokens
                        .iter()
                        .map(String::as_str)
                        .filter(|t| {
                            !matches!(
                                t.to_lowercase().as_str(),
                                "namespace" | "module" | "assembly"
                            )
                        })
                        .collect();
                    if let Some(last) = module_tokens.last() {
                        let module_name = last.split('.').next_back().unwrap_or("").to_string();
                        if !module_name.is_empty() {
                            let tgt_nid = make_id1(&module_name);
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
                    }
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_ps(
                        cur.node(),
                        source,
                        str_path,
                        stem,
                        file_nid,
                        parent_class_nid,
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

#[allow(clippy::too_many_arguments)]
fn walk_calls_ps(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
    label_to_nid: &HashMap<String, String>,
    edges: &mut Vec<Edge>,
    seen_call_pairs: &mut HashSet<(String, String)>,
    raw_calls: &mut Vec<RawCall>,
) {
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if matches!(node.kind(), "function_statement" | "class_statement") {
        return;
    }

    if node.kind() == "command" {
        let cmd_name_node = {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                let mut found = None;
                loop {
                    if cur.node().kind() == "command_name" {
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
        if let Some(nn) = cmd_name_node {
            let cmd_text = read_text(nn, source);
            if !PS_SKIP.contains(cmd_text.to_lowercase().as_str()) {
                let tgt_nid = label_to_nid.get(&cmd_text.to_lowercase()).cloned();
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
                } else if !cmd_text.is_empty() {
                    raw_calls.push(RawCall {
                        caller_nid: caller_nid.to_string(),
                        callee: cmd_text.to_string(),
                        is_member_call: false,
                        source_file: str_path.to_string(),
                        source_location: format!("L{}", node.start_position().row + 1),
                    });
                }
            }
        }
    }

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_ps(
                cur.node(),
                source,
                str_path,
                caller_nid,
                body_start,
                body_end,
                label_to_nid,
                edges,
                seen_call_pairs,
                raw_calls,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
