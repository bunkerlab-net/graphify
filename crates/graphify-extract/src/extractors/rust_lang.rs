//! Rust extractor — custom walk over tree-sitter-rust AST.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};

/// Common Rust trait/stdlib method names that appear in virtually every codebase.
/// Resolving these cross-file produces spurious INFERRED edges — skip them from
/// the unresolved-call queue entirely.
static RUST_TRAIT_METHOD_BLOCKLIST: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "new",
        "default",
        "parse",
        "from_str",
        "now",
        "clone",
        "into",
        "from",
        "to_string",
        "to_owned",
        "len",
        "is_empty",
        "iter",
        "next",
        "build",
        "start",
        "run",
        "init",
        "app",
        "get",
        "set",
        "push",
        "pop",
        "insert",
        "remove",
        "contains",
        "collect",
        "map",
        "filter",
        "unwrap",
        "expect",
        "ok",
        "err",
        "some",
        "none",
        "send",
        "recv",
        "lock",
        "read",
        "write",
    ]
    .into_iter()
    .collect()
});

fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, structs, enums, traits, impl methods, and use declarations from a `.rs` file.
#[must_use]
pub fn extract_rust(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set rust language".to_string()),
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
    walk_rust(
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

    // Build label→nid map for intra-file call resolution
    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();

    for (caller_nid, body_start, body_end) in &function_bodies {
        let root2 = tree.root_node();
        walk_calls_rust(
            root2,
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_rust(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    file_nid: &str,
    parent_impl_nid: Option<&str>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, usize, usize)>,
) {
    let t = node.kind();

    match t {
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let func_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let (func_nid, label, parent) = if let Some(impl_nid) = parent_impl_nid {
                    (
                        make_id(&[impl_nid, func_name]),
                        format!(".{func_name}()"),
                        impl_nid.to_string(),
                    )
                } else {
                    (
                        make_id(&[stem, func_name]),
                        format!("{func_name}()"),
                        file_nid.to_string(),
                    )
                };
                let relation = if parent_impl_nid.is_some() {
                    "method"
                } else {
                    "contains"
                };
                if seen_ids.insert(func_nid.clone()) {
                    nodes.push(Node {
                        id: func_nid.clone(),
                        label,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
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
        "struct_item" | "enum_item" | "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let item_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let item_nid = make_id(&[stem, item_name]);
                if seen_ids.insert(item_nid.clone()) {
                    nodes.push(Node {
                        id: item_nid.clone(),
                        label: item_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: item_nid,
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
            }
        }
        "impl_item" => {
            let mut impl_nid: Option<String> = None;
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = read_text(type_node, source).trim().to_string();
                let nid = make_id(&[stem, &type_name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(nid.clone()) {
                    nodes.push(Node {
                        id: nid.clone(),
                        label: type_name,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                    });
                }
                impl_nid = Some(nid);
            }
            if let Some(body) = node.child_by_field_name("body") {
                let mut cur = body.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_rust(
                            cur.node(),
                            source,
                            str_path,
                            stem,
                            file_nid,
                            impl_nid.as_deref(),
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
        "use_declaration" => {
            if let Some(arg) = node.child_by_field_name("argument") {
                let raw = read_text(arg, source);
                let clean = raw
                    .split('{')
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(':')
                    .trim_end_matches('*')
                    .trim_end_matches(':')
                    .to_string();
                let module_name = clean.split("::").last().unwrap_or("").trim().to_string();
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
                        context: Some("import".to_string()),
                        confidence_score: None,
                    });
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_rust(
                        cur.node(),
                        source,
                        str_path,
                        stem,
                        file_nid,
                        // Don't propagate impl_nid through generic nodes
                        None,
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
fn walk_calls_rust(
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
    if node.kind() == "function_item" {
        return;
    }

    if node.kind() == "call_expression"
        && let Some(func_node) = node.child_by_field_name("function")
    {
        let mut callee_name: Option<String> = None;
        let mut is_member_call = false;
        let mut is_scoped_call = false;
        match func_node.kind() {
            "identifier" => {
                callee_name = Some(read_text(func_node, source).to_string());
            }
            "field_expression" => {
                is_member_call = true;
                if let Some(field) = func_node.child_by_field_name("field") {
                    callee_name = Some(read_text(field, source).to_string());
                }
            }
            "scoped_identifier" => {
                is_scoped_call = true;
                if let Some(name) = func_node.child_by_field_name("name") {
                    callee_name = Some(read_text(name, source).to_string());
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
            } else if !is_scoped_call
                && !RUST_TRAIT_METHOD_BLOCKLIST.contains(cn.to_lowercase().as_str())
            {
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

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_rust(
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
