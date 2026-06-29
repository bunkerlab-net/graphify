//! Elixir extractor — custom walk over tree-sitter-elixir AST.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use crate::ids::file_stem;
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};

static IMPORT_KEYWORDS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| ["alias", "import", "require", "use"].into_iter().collect());

static SKIP_KEYWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "def",
        "defp",
        "defmodule",
        "defmacro",
        "defmacrop",
        "defstruct",
        "defprotocol",
        "defimpl",
        "defguard",
        "alias",
        "import",
        "require",
        "use",
        "if",
        "unless",
        "case",
        "cond",
        "with",
        "for",
    ]
    .into_iter()
    .collect()
});

/// Return the source bytes covered by `node` as an owned `String` (lossy UTF-8).
fn read_text(node: tree_sitter::Node<'_>, source: &[u8]) -> String {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

/// Extract modules, functions, imports, and calls from a `.ex`/`.exs` file.
#[must_use]
// Single-pass tree-sitter extractor: node/edge emission shares accumulator
// state across module/function/call branches, so splitting into helpers
// would separate related logic.
#[allow(clippy::too_many_lines)]
pub fn extract_elixir(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_elixir::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set elixir language".to_string()),
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
        origin_file: None,
    });

    let root = tree.root_node();
    {
        let mut walk_ctx = ElixirWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
        };
        walk_elixir(&mut walk_ctx, root, &source, None);
    }

    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    {
        let mut call_ctx = ElixirCallCtx {
            str_path: &str_path,
            label_to_nid: &label_to_nid,
            edges: &mut edges,
            seen_call_pairs: &mut seen_call_pairs,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_start, body_end) in &function_bodies {
            walk_calls_elixir(
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
                && (seen_ids.contains(&e.target) || e.relation == "imports")
        })
        .collect();

    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}

/// Extract the text of the first `alias` child node, used to resolve Elixir `alias` expressions.
fn get_alias_text(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return None;
    }
    loop {
        if cur.node().kind() == "alias" {
            return Some(read_text(cur.node(), source));
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Recursively walk an Elixir AST emitting nodes and edges for modules, functions, and imports.
///
/// Handles `defmodule`, `def`/`defp`, `alias`/`import`/`require`/`use` call expressions.
/// Descends into nested modules. Mirrors Python `_walk_elixir`.
/// Shared state threaded through every [`walk_elixir`] recursion.
struct ElixirWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    function_bodies: &'a mut Vec<(String, usize, usize)>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Elixir's call/macro node shapes
fn walk_elixir(
    ctx: &mut ElixirWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    parent_module_nid: Option<&str>,
) {
    let str_path = ctx.str_path;
    let stem = ctx.stem;
    let file_nid = ctx.file_nid;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let function_bodies = &mut *ctx.function_bodies;
    if node.kind() != "call" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                walk_elixir(ctx, cur.node(), source, parent_module_nid);
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // It's a call node — extract identifier, arguments, do_block
    let mut identifier_node: Option<tree_sitter::Node<'_>> = None;
    let mut arguments_node: Option<tree_sitter::Node<'_>> = None;
    let mut do_block_node: Option<tree_sitter::Node<'_>> = None;
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            match child.kind() {
                "identifier" => identifier_node = Some(child),
                "arguments" => arguments_node = Some(child),
                "do_block" => do_block_node = Some(child),
                _ => {}
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    let Some(ident_node) = identifier_node else {
        let mut cur2 = node.walk();
        if cur2.goto_first_child() {
            loop {
                walk_elixir(ctx, cur2.node(), source, parent_module_nid);
                if !cur2.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    };

    let keyword = read_text(ident_node, source);
    let line = node.start_position().row + 1;

    match keyword.as_str() {
        "defmodule" => {
            let module_name = arguments_node.and_then(|a| get_alias_text(a, source));
            let Some(mn) = module_name else { return };
            let module_nid = make_id(&[stem, &mn]);
            if seen_ids.insert(module_nid.clone()) {
                nodes.push(Node {
                    id: module_nid.clone(),
                    label: mn,
                    file_type: "code".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    metadata: None,
                    origin_file: None,
                });
            }
            edges.push(Edge {
                external: false,
                source: file_nid.to_string(),
                target: module_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            if let Some(do_block) = do_block_node {
                let mut c = do_block.walk();
                if c.goto_first_child() {
                    loop {
                        walk_elixir(ctx, c.node(), source, Some(&module_nid));
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "def" | "defp" => {
            let mut func_name: Option<String> = None;
            if let Some(args_node) = arguments_node {
                let mut ac = args_node.walk();
                if ac.goto_first_child() {
                    loop {
                        let child = ac.node();
                        if child.kind() == "call" {
                            let mut sc = child.walk();
                            if sc.goto_first_child() {
                                loop {
                                    if sc.node().kind() == "identifier" {
                                        func_name = Some(read_text(sc.node(), source));
                                        break;
                                    }
                                    if !sc.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                        } else if child.kind() == "identifier" {
                            func_name = Some(read_text(child, source));
                            break;
                        }
                        if func_name.is_some() {
                            break;
                        }
                        if !ac.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            let Some(fn_name) = func_name else { return };
            let container = parent_module_nid.unwrap_or(file_nid);
            let func_nid = make_id(&[container, &fn_name]);
            if seen_ids.insert(func_nid.clone()) {
                nodes.push(Node {
                    id: func_nid.clone(),
                    label: format!("{fn_name}()"),
                    file_type: "code".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    metadata: None,
                    origin_file: None,
                });
            }
            let relation = if parent_module_nid.is_some() {
                "method"
            } else {
                "contains"
            };
            edges.push(Edge {
                external: false,
                source: container.to_string(),
                target: func_nid.clone(),
                relation: relation.to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            if let Some(do_block) = do_block_node {
                function_bodies.push((func_nid, do_block.start_byte(), do_block.end_byte()));
            }
        }
        kw if IMPORT_KEYWORDS.contains(kw) => {
            if let Some(args_node) = arguments_node {
                let module_name = get_alias_text(args_node, source);
                if let Some(mn) = module_name {
                    let tgt_nid = make_id1(&mn);
                    edges.push(Edge {
                        external: false,
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
        _ => {
            let mut cur2 = node.walk();
            if cur2.goto_first_child() {
                loop {
                    walk_elixir(ctx, cur2.node(), source, parent_module_nid);
                    if !cur2.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` edges within an Elixir function body.
///
/// Recurses through the body's AST, skipping nested `def`/`defp` definitions, and emits
/// `calls` edges for `call_expression` nodes whose callee matches a known function NID.
/// Elixir Kernel built-ins in `SKIP_KEYWORDS` are filtered out. Mirrors Python `_walk_calls_elixir`.
/// Shared state threaded through every [`walk_calls_elixir`] recursion.
struct ElixirCallCtx<'a> {
    str_path: &'a str,
    label_to_nid: &'a HashMap<String, String>,
    edges: &'a mut Vec<Edge>,
    seen_call_pairs: &'a mut HashSet<(String, String)>,
    raw_calls: &'a mut Vec<RawCall>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Elixir's call-site shapes
fn walk_calls_elixir(
    ctx: &mut ElixirCallCtx<'_>,
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
    if node.kind() != "call" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                walk_calls_elixir(ctx, cur.node(), source, caller_nid, body_start, body_end);
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // Check if the call is a skip keyword
    let mut first_ident: Option<String> = None;
    {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "identifier" {
                    first_ident = Some(read_text(cur.node(), source));
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    if let Some(ref kw) = first_ident
        && SKIP_KEYWORDS.contains(kw.as_str())
    {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                walk_calls_elixir(ctx, cur.node(), source, caller_nid, body_start, body_end);
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    let mut callee_name: Option<String> = None;
    let mut is_member_call = false;
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "dot" {
                is_member_call = true;
                let dot_text = read_text(child, source);
                let parts: Vec<&str> = dot_text.trim_end_matches('.').split('.').collect();
                if let Some(last) = parts.last() {
                    callee_name = Some((*last).to_string());
                }
                break;
            }
            if child.kind() == "identifier" {
                callee_name = Some(read_text(child, source));
                break;
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    if let Some(cn) = callee_name
        && !crate::builtins::is_language_builtin_global(&cn)
    {
        let tgt_nid = label_to_nid.get(&cn.to_lowercase()).cloned();
        if let Some(tgt) = tgt_nid {
            if tgt != caller_nid {
                let pair = (caller_nid.to_string(), tgt.clone());
                if seen_call_pairs.insert(pair) {
                    let line = node.start_position().row + 1;
                    edges.push(Edge {
                        external: false,
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
                receiver: None,
                receiver_type: None,
            });
        }
    }

    let mut cur2 = node.walk();
    if cur2.goto_first_child() {
        loop {
            walk_calls_elixir(ctx, cur2.node(), source, caller_nid, body_start, body_end);
            if !cur2.goto_next_sibling() {
                break;
            }
        }
    }
}
