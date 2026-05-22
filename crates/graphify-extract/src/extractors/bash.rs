//! Bash extractor — custom walk over tree-sitter-bash AST.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

static BASH_SKIP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
        "in", "return", "exit", "break", "continue", "echo", "printf", "cd", "set", "local",
        "export", "readonly", "declare", "unset", "shift", "read", "test", "[", "[[", ":", "true",
        "false", "source", ".", "trap", "wait", "exec", "eval",
    ]
    .into_iter()
    .collect()
});

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, source imports, and cross-function calls from a `.sh` file.
#[must_use]
pub fn extract_bash(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set bash language".to_string()),
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
    let mut defined_functions: HashSet<String> = HashSet::new();

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
    walk_bash(
        root,
        &source,
        &str_path,
        &stem,
        &file_nid,
        path,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut function_bodies,
        &mut defined_functions,
    );

    // Second pass: cross-function calls
    for (fn_nid, body_start, body_end) in &function_bodies {
        let mut seen_calls: HashSet<(String, String)> = HashSet::new();
        walk_calls_bash(
            tree.root_node(),
            &source,
            &str_path,
            &stem,
            fn_nid,
            *body_start,
            *body_end,
            &defined_functions,
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

/// Recursively walk a Bash AST, emitting nodes and edges for functions and `source` imports.
///
/// Handles `function_definition` (named Bash functions), `command` nodes whose name is `source`
/// or `.` (treated as file imports), and descends into all child nodes. Mirrors Python
/// `_walk_bash`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_bash(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    file_nid: &str,
    path: &Path,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
    defined_functions: &mut HashSet<String>,
) {
    let t = node.kind();

    match t {
        "function_definition" => {
            // bash grammar: function_definition has a word child (the name)
            let name = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "word" {
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
            if let Some(func_name) = name {
                let fn_nid = make_id(&[stem, &func_name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(fn_nid.clone()) {
                    nodes.push(Node {
                        id: fn_nid.clone(),
                        label: format!("{func_name}()"),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: fn_nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                defined_functions.insert(func_name);
                // find the compound_statement body
                let body = {
                    let mut cur = node.walk();
                    if cur.goto_first_child() {
                        let mut f = None;
                        loop {
                            if cur.node().kind() == "compound_statement" {
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
                if let Some(b) = body {
                    function_bodies.push((fn_nid, b.start_byte(), b.end_byte()));
                }
                // don't recurse into function body during structural pass
            }
        }
        "command" => {
            let cmd_name_node = node.child_by_field_name("name").or_else(|| {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    Some(cur.node())
                } else {
                    None
                }
            });
            if let Some(cnn) = cmd_name_node {
                let cmd = read_text(cnn, source).trim().to_string();
                if matches!(cmd.as_str(), "source" | ".") {
                    // find path argument
                    let args: Vec<tree_sitter::Node<'_>> = {
                        let mut a = vec![];
                        let mut cur = node.walk();
                        if cur.goto_first_child() {
                            loop {
                                let child = cur.node();
                                if matches!(child.kind(), "word" | "string" | "concatenation")
                                    && child.start_byte() != cnn.start_byte()
                                {
                                    a.push(child);
                                }
                                if !cur.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                        a
                    };
                    if let Some(arg) = args.first() {
                        let raw = read_text(*arg, source)
                            .trim()
                            .trim_matches(|c| c == '\'' || c == '"')
                            .to_string();
                        let line = node.start_position().row + 1;
                        if raw.starts_with('.') || raw.starts_with('/') {
                            let resolved = path
                                .parent()
                                .map(|p| p.join(&raw))
                                .and_then(|p| p.canonicalize().ok());
                            if let Some(res) = resolved {
                                let tgt_nid = make_id1(&res.to_string_lossy());
                                edges.push(Edge {
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
                            }
                        } else {
                            let tgt_nid = make_id1(&raw);
                            if !tgt_nid.is_empty() {
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
                    }
                }
            }
        }
        "declaration_command" => {
            // export/declare/readonly VAR=value at program level
            let is_top_level = node.parent().is_some_and(|p| p.kind() == "program");
            if is_top_level {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "variable_assignment"
                            && let Some(var_node) = cur.node().child_by_field_name("name")
                        {
                            let var = read_text(var_node, source).trim().to_string();
                            if !var.is_empty() {
                                let var_nid = make_id(&[stem, &var]);
                                let line = cur.node().start_position().row + 1;
                                if seen_ids.insert(var_nid.clone()) {
                                    nodes.push(Node {
                                        id: var_nid.clone(),
                                        label: var,
                                        file_type: "code".to_string(),
                                        source_file: str_path.to_string(),
                                        source_location: Some(format!("L{line}")),
                                    });
                                }
                                edges.push(Edge {
                                    source: file_nid.to_string(),
                                    target: var_nid,
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
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_bash(
                        cur.node(),
                        source,
                        str_path,
                        stem,
                        file_nid,
                        path,
                        nodes,
                        edges,
                        seen_ids,
                        function_bodies,
                        defined_functions,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` edges from within a Bash function body.
///
/// Recursively descends the AST looking for `command` nodes. When the command name is a known
/// function in this file (via `label_to_nid`), a `calls` edge is emitted. Bash built-ins and
/// control-flow keywords are filtered via `BASH_SKIP`. Mirrors Python `_walk_calls_bash`.
#[allow(clippy::too_many_arguments)]
fn walk_calls_bash(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    func_nid: &str,
    body_start: usize,
    body_end: usize,
    defined_functions: &HashSet<String>,
    edges: &mut Vec<Edge>,
    seen_calls: &mut HashSet<(String, String)>,
) {
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if node.kind() == "command" {
        let cmd_name_node = node.child_by_field_name("name").or_else(|| {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                Some(cur.node())
            } else {
                None
            }
        });
        if let Some(cnn) = cmd_name_node {
            let name = read_text(cnn, source).trim().to_string();
            if !name.is_empty()
                && !BASH_SKIP.contains(name.as_str())
                && defined_functions.contains(&name)
            {
                let tgt = make_id(&[stem, &name]);
                let key = (func_nid.to_string(), tgt.clone());
                if !tgt.is_empty() && !seen_calls.contains(&key) {
                    seen_calls.insert(key);
                    let line = node.start_position().row + 1;
                    edges.push(Edge {
                        source: func_nid.to_string(),
                        target: tgt,
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
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_bash(
                cur.node(),
                source,
                str_path,
                stem,
                func_nid,
                body_start,
                body_end,
                defined_functions,
                edges,
                seen_calls,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
