//! Go extractor — custom walk over tree-sitter-go AST.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, methods, type declarations, and imports from a `.go` file.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn extract_go(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set go language".to_string()),
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
    // Use directory name as package scope so methods on the same type across
    // multiple files in a package share one canonical type node.
    let pkg_scope = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&stem)
        .to_string();
    let str_path = path.to_string_lossy().into_owned();

    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut function_bodies: Vec<(String, usize, usize)> = Vec::new();
    let mut go_imported_pkgs: HashSet<String> = HashSet::new();

    macro_rules! add_node {
        ($nid:expr, $label:expr, $line:expr) => {
            let nid: String = $nid;
            if seen_ids.insert(nid.clone()) {
                nodes.push(Node {
                    id: nid,
                    label: $label,
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{}", $line)),
                });
            }
        };
    }

    let file_nid = make_id1(&str_path);
    add_node!(
        file_nid.clone(),
        path.file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        1
    );

    // Structural walk
    let root = tree.root_node();
    walk_go(
        root,
        &source,
        &str_path,
        &stem,
        &pkg_scope,
        &file_nid,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
        &mut function_bodies,
        &mut go_imported_pkgs,
    );

    // Build label→nid map for intra-file call resolution
    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();

    // Second pass: calls inside function/method bodies
    for (caller_nid, body_start, body_end) in &function_bodies {
        let body_bytes = &source[*body_start..*body_end];
        // Re-parse just the body range via cursor walk
        let body_root = tree.root_node();
        walk_calls_go(
            body_root,
            &source,
            &str_path,
            caller_nid,
            *body_start,
            *body_end,
            &label_to_nid,
            &go_imported_pkgs,
            &mut edges,
            &mut seen_call_pairs,
            &mut raw_calls,
        );
        let _ = body_bytes; // suppress unused warning
    }

    // Filter edges: src must be valid, tgt must be valid OR relation is import
    let valid_ids = &seen_ids;
    let clean_edges: Vec<Edge> = edges
        .into_iter()
        .filter(|e| {
            valid_ids.contains(&e.source)
                && (valid_ids.contains(&e.target)
                    || matches!(e.relation.as_str(), "imports" | "imports_from"))
        })
        .collect();

    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}

/// Recursively walk a Go AST emitting nodes and edges for functions, methods, and type declarations.
///
/// Handles `function_declaration`, `method_declaration`, `type_declaration`, and `import_declaration`
/// nodes. Descends into all child nodes. Mirrors Python `_walk_go`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_go(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    pkg_scope: &str,
    file_nid: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
    go_imported_pkgs: &mut HashSet<String>,
) {
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
                        });
                    }
                    edges.push(Edge {
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
                        });
                    }
                    edges.push(Edge {
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
                    walk_go(
                        cur.node(),
                        source,
                        str_path,
                        stem,
                        pkg_scope,
                        file_nid,
                        nodes,
                        edges,
                        seen_ids,
                        function_bodies,
                        go_imported_pkgs,
                    );
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

/// Collect `calls` edges within a Go function or method body.
///
/// Recurses through the body AST, emitting `calls` edges for `call_expression` nodes whose
/// callee matches a known function NID in this file. Selector expressions (package.Func) are
/// resolved against `go_imported_pkgs`. Mirrors Python `_walk_calls_go`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_calls_go(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
    label_to_nid: &HashMap<String, String>,
    go_imported_pkgs: &HashSet<String>,
    edges: &mut Vec<Edge>,
    seen_call_pairs: &mut HashSet<(String, String)>,
    raw_calls: &mut Vec<RawCall>,
) {
    // Only visit nodes within the body range
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }

    match node.kind() {
        "function_declaration" | "method_declaration" => {
            // Don't recurse into nested functions
        }
        "call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let mut callee_name: Option<String> = None;
                let mut is_member_call = false;
                match func_node.kind() {
                    "identifier" => {
                        callee_name = Some(read_text(func_node, source).to_string());
                    }
                    "selector_expression" => {
                        let field = func_node.child_by_field_name("field");
                        let operand = func_node.child_by_field_name("operand");
                        let receiver_name = operand
                            .map(|n| read_text(n, source).to_string())
                            .unwrap_or_default();
                        // Package-qualified call: fmt.Println → not a member call
                        is_member_call = !go_imported_pkgs.contains(&receiver_name);
                        if let Some(f) = field {
                            callee_name = Some(read_text(f, source).to_string());
                        }
                    }
                    _ => {}
                }
                if let Some(cn) = callee_name {
                    let tgt_nid = label_to_nid.get(&cn.to_lowercase()).cloned();
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
                                    context: Some("call".to_string()),
                                    confidence_score: None,
                                });
                            }
                        }
                    } else {
                        raw_calls.push(RawCall {
                            caller_nid: caller_nid.to_string(),
                            callee: cn,
                            is_member_call,
                            source_file: str_path.to_string(),
                            source_location: format!("L{}", node.start_position().row + 1),
                        });
                    }
                }
            }
            // Recurse into arguments
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_calls_go(
                        cur.node(),
                        source,
                        str_path,
                        caller_nid,
                        body_start,
                        body_end,
                        label_to_nid,
                        go_imported_pkgs,
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
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_calls_go(
                        cur.node(),
                        source,
                        str_path,
                        caller_nid,
                        body_start,
                        body_end,
                        label_to_nid,
                        go_imported_pkgs,
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
    }
}
