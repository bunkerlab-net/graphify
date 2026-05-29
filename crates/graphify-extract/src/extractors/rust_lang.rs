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

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, structs, enums, traits, impl methods, and use declarations from a `.rs` file.
#[must_use]
pub fn extract_rust(path: &Path) -> FileResult {
    let Some((source, tree)) = parse_rust_source(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("parse failed".to_string()),
        };
    };
    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    let mut nodes: Vec<Node> = vec![Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::from([file_nid.clone()]);
    let mut function_bodies: Vec<(String, usize, usize)> = Vec::new();

    {
        let mut walk_ctx = RustWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
        };
        walk_rust(&mut walk_ctx, tree.root_node(), &source, None);
    }

    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    {
        let mut call_ctx = RustCallCtx {
            str_path: &str_path,
            label_to_nid: &label_to_nid,
            edges: &mut edges,
            seen_call_pairs: &mut seen_call_pairs,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_start, body_end) in &function_bodies {
            walk_calls_rust(
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
                && (seen_ids.contains(&e.target)
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

/// Read + tree-sitter-parse a `.rs` file. `None` on any I/O or parse error.
fn parse_rust_source(path: &Path) -> Option<(Vec<u8>, tree_sitter::Tree)> {
    let source = std::fs::read(path).ok()?;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(&source, None)?;
    Some((source, tree))
}

/// Recursively walk a Rust AST emitting nodes for functions, structs, enums, traits, and impls.
///
/// Records function body byte ranges for the subsequent call-graph pass. Handles `use_declaration`
/// to produce import edges. Mirrors Python `_walk_rust`.
/// Shared state threaded through every [`walk_rust`] recursion.
struct RustWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    function_bodies: &'a mut Vec<(String, usize, usize)>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Rust's AST node kinds
fn walk_rust(
    ctx: &mut RustWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    parent_impl_nid: Option<&str>,
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
                        make_id(&[ctx.stem, func_name]),
                        format!("{func_name}()"),
                        ctx.file_nid.to_string(),
                    )
                };
                let relation = if parent_impl_nid.is_some() {
                    "method"
                } else {
                    "contains"
                };
                if ctx.seen_ids.insert(func_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: func_nid.clone(),
                        label,
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: parent,
                    target: func_nid.clone(),
                    relation: relation.to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                if let Some(body) = node.child_by_field_name("body") {
                    ctx.function_bodies
                        .push((func_nid, body.start_byte(), body.end_byte()));
                }
            }
        }
        "struct_item" | "enum_item" | "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let item_name = read_text(name_node, source);
                let line = node.start_position().row + 1;
                let item_nid = make_id(&[ctx.stem, item_name]);
                if ctx.seen_ids.insert(item_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: item_nid.clone(),
                        label: item_name.to_string(),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: ctx.file_nid.to_string(),
                    target: item_nid,
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
        "impl_item" => {
            let mut impl_nid: Option<String> = None;
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = read_text(type_node, source).trim().to_string();
                let nid = make_id(&[ctx.stem, &type_name]);
                let line = node.start_position().row + 1;
                if ctx.seen_ids.insert(nid.clone()) {
                    ctx.nodes.push(Node {
                        id: nid.clone(),
                        label: type_name,
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                impl_nid = Some(nid);
            }
            if let Some(body) = node.child_by_field_name("body") {
                let mut cur = body.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_rust(ctx, cur.node(), source, impl_nid.as_deref());
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
                    ctx.edges.push(Edge {
                        external: false,
                        source: ctx.file_nid.to_string(),
                        target: tgt_nid,
                        relation: "imports_from".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
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
                        ctx,
                        cur.node(),
                        source, // Don't propagate impl_nid through generic ctx.nodes
                        None,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` ctx.edges within a Rust function body's byte range.
///
/// Recurses through the body AST, emitting `calls` ctx.edges for `call_expression` and
/// `macro_invocation` ctx.nodes whose callee matches a known NID. Mirrors Python `_walk_calls_rust`.
/// Shared state threaded through every [`walk_calls_rust`] recursion.
struct RustCallCtx<'a> {
    str_path: &'a str,
    label_to_nid: &'a HashMap<String, String>,
    edges: &'a mut Vec<Edge>,
    seen_call_pairs: &'a mut HashSet<(String, String)>,
    raw_calls: &'a mut Vec<RawCall>,
}

fn walk_calls_rust(
    ctx: &mut RustCallCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
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
            // Resolve first so a built-in name backing a real local symbol is
            // kept; only drop unresolved built-ins (god-node guard, #726).
            let tgt_nid = ctx.label_to_nid.get(&cn.to_lowercase()).cloned();
            if let Some(tgt) = tgt_nid {
                if tgt != caller_nid {
                    let pair = (caller_nid.to_string(), tgt.clone());
                    if ctx.seen_call_pairs.insert(pair) {
                        let line = node.start_position().row + 1;
                        ctx.edges.push(Edge {
                            external: false,
                            source: caller_nid.to_string(),
                            target: tgt,
                            relation: "calls".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: Some("call".to_string()),
                            confidence_score: None,
                        });
                    }
                }
            } else if !is_scoped_call
                && !RUST_TRAIT_METHOD_BLOCKLIST.contains(cn.to_lowercase().as_str())
                && !crate::builtins::is_language_builtin_global(&cn)
            {
                ctx.raw_calls.push(RawCall {
                    caller_nid: caller_nid.to_string(),
                    callee: cn,
                    is_member_call,
                    source_file: ctx.str_path.to_string(),
                    source_location: format!("L{}", node.start_position().row + 1),
                });
            }
        }
    }

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_rust(ctx, cur.node(), source, caller_nid, body_start, body_end);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
