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

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, classes, methods, and using statements from a `.ps1` file.
#[must_use]
// Single-pass tree-sitter extractor: node/edge emission shares accumulator
// state across function/class/method branches, so splitting fragments locality.
#[allow(clippy::too_many_lines)]
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
        metadata: None,
    });

    let root = tree.root_node();
    {
        let mut walk_ctx = PsWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
        };
        walk_ps(&mut walk_ctx, root, &source, None);
    }

    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    {
        let mut call_ctx = PsCallCtx {
            str_path: &str_path,
            label_to_nid: &label_to_nid,
            edges: &mut edges,
            seen_call_pairs: &mut seen_call_pairs,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_start, body_end) in &function_bodies {
            walk_calls_ps(
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

/// Locate the `script_block_body` (or `script_block`) inside a function's body node.
///
/// PowerShell function bodies are wrapped in a `script_block` container; this helper peels
/// that wrapper so callers receive the actual statement list for call-graph walking.
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

/// Recursively walk a PowerShell AST emitting nodes for functions, classes, and methods.
///
/// Handles `function_definition`, `class_statement`, `method_statement`, and `using_statement`
/// nodes. Mirrors Python `_walk_ps`.
/// Shared state threaded through every [`walk_ps`] recursion.
struct PsWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    function_bodies: &'a mut Vec<(String, usize, usize)>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over PowerShell's AST node kinds
fn walk_ps(
    ctx: &mut PsWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    parent_class_nid: Option<&str>,
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
                        metadata: None,
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
                        metadata: None,
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
                        walk_ps(ctx, cur.node(), source, Some(&class_nid));
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
                        metadata: None,
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
                    walk_ps(ctx, cur.node(), source, parent_class_nid);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` edges within a PowerShell function body.
///
/// Recurses through the body AST, emitting `calls` edges for `command` and `invocation_expression`
/// nodes whose callee matches a known function NID. Mirrors Python `_walk_calls_ps`.
/// Shared state threaded through every [`walk_calls_ps`] recursion.
struct PsCallCtx<'a> {
    str_path: &'a str,
    label_to_nid: &'a HashMap<String, String>,
    edges: &'a mut Vec<Edge>,
    seen_call_pairs: &'a mut HashSet<(String, String)>,
    raw_calls: &'a mut Vec<RawCall>,
}

fn walk_calls_ps(
    ctx: &mut PsCallCtx<'_>,
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
            walk_calls_ps(ctx, cur.node(), source, caller_nid, body_start, body_end);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
