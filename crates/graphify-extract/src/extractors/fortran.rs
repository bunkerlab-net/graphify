//! Fortran extractor — custom walk over tree-sitter-fortran AST.

// Tree-sitter row numbers are source line indices; no file has 2^32 lines.
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashSet;
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

static FORTRAN_CPP_EXTS: &[&str] = &[".F", ".F90", ".F95", ".F03", ".F08"];

const STMT_HEADERS: &[&str] = &[
    "subroutine_statement",
    "function_statement",
    "program_statement",
    "module_statement",
];

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Run the C preprocessor on a Fortran file to expand macros and `#include` directives.
///
/// Used for free-form Fortran files with `.F` / `.F90` / etc. extensions that use the
/// C preprocessor. Falls back to reading the raw bytes if `cpp` is unavailable or fails.
fn cpp_preprocess(path: &Path) -> Vec<u8> {
    // Security: pass -nostdinc -I /dev/null to prevent file exfiltration
    let result = std::process::Command::new("cpp")
        .args(["-w", "-P", "-nostdinc", "-I", "/dev/null"])
        .arg(path)
        .output();
    match result {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => out.stdout,
        _ => std::fs::read(path).unwrap_or_default(),
    }
}

/// Extract programs, modules, subroutines, functions, use statements, and calls from Fortran files.
#[must_use]
pub fn extract_fortran(path: &Path) -> FileResult {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dot_ext = format!(".{ext}");
    let source = if FORTRAN_CPP_EXTS.contains(&dot_ext.as_str()) {
        cpp_preprocess(path)
    } else {
        match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                return FileResult {
                    nodes: vec![],
                    edges: vec![],
                    raw_calls: vec![],
                    error: Some(e.to_string()),
                };
            }
        }
    };

    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_fortran::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set fortran language".to_string()),
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
    let mut scope_bodies: Vec<(String, usize, usize)> = Vec::new();

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
        let mut walk_ctx = FortranWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            scope_bodies: &mut scope_bodies,
        };
        walk_fortran(&mut walk_ctx, root, &source, &file_nid);
    }

    // Call pass
    {
        let mut call_ctx = FortranCallCtx {
            str_path: &str_path,
            stem: &stem,
            stmt_headers: STMT_HEADERS,
            edges: &mut edges,
        };
        for (scope_nid, body_start, body_end) in &scope_bodies {
            walk_calls_fortran(
                &mut call_ctx,
                tree.root_node(),
                &source,
                scope_nid,
                *body_start,
                *body_end,
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

/// Extract the lowercased `name` or `identifier` child from a Fortran statement node.
///
/// Used to pull the declared name from `subroutine_statement`, `function_statement`, etc.
fn fortran_name(stmt_node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = stmt_node.walk();
    if cur.goto_first_child() {
        loop {
            if matches!(cur.node().kind(), "name" | "identifier") {
                return Some(read_text(cur.node(), source).to_lowercase());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Recursively walk a Fortran AST emitting nodes for programs, modules, subroutines, and functions.
///
/// Records byte ranges of scope bodies for use by `walk_calls_fortran`. Mirrors Python `_walk_fortran`.
/// Shared state threaded through every [`walk_fortran`] recursion.
struct FortranWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    scope_bodies: &'a mut Vec<(String, usize, usize)>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Fortran's AST node kinds
fn walk_fortran(
    ctx: &mut FortranWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    scope_nid: &str,
) {
    let str_path = ctx.str_path;
    let stem = ctx.stem;
    let file_nid = ctx.file_nid;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let scope_bodies = &mut *ctx.scope_bodies;
    let t = node.kind();

    match t {
        "program" => {
            let stmt = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "program_statement" {
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
            if let Some(name) = stmt.and_then(|s| fortran_name(s, source)) {
                let nid = make_id(&[stem, &name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(nid.clone()) {
                    nodes.push(Node {
                        id: nid.clone(),
                        label: name,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                scope_bodies.push((nid.clone(), node.start_byte(), node.end_byte()));
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_fortran(ctx, cur.node(), source, &nid);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "module" => {
            let stmt = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "module_statement" {
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
            if let Some(name) = stmt.and_then(|s| fortran_name(s, source)) {
                let nid = make_id(&[stem, &name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(nid.clone()) {
                    nodes.push(Node {
                        id: nid.clone(),
                        label: name,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    source: file_nid.to_string(),
                    target: nid.clone(),
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
                        walk_fortran(ctx, cur.node(), source, &nid);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "subroutine" => {
            let stmt = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "subroutine_statement" {
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
            if let Some(name) = stmt.and_then(|s| fortran_name(s, source)) {
                let nid = make_id(&[stem, &name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(nid.clone()) {
                    nodes.push(Node {
                        id: nid.clone(),
                        label: format!("{name}()"),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    source: scope_nid.to_string(),
                    target: nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                scope_bodies.push((nid.clone(), node.start_byte(), node.end_byte()));
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_fortran(ctx, cur.node(), source, &nid);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "function" => {
            let stmt = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "function_statement" {
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
            if let Some(name) = stmt.and_then(|s| fortran_name(s, source)) {
                let nid = make_id(&[stem, &name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(nid.clone()) {
                    nodes.push(Node {
                        id: nid.clone(),
                        label: format!("{name}()"),
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    source: scope_nid.to_string(),
                    target: nid.clone(),
                    relation: "defines".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                scope_bodies.push((nid.clone(), node.start_byte(), node.end_byte()));
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_fortran(ctx, cur.node(), source, &nid);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "use_statement" => {
            let line = node.start_position().row + 1;
            let name_node = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if matches!(cur.node().kind(), "module_name" | "name" | "identifier") {
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
            if let Some(nn) = name_node {
                let mod_name = read_text(nn, source).to_lowercase();
                let imp_nid = make_id1(&mod_name);
                seen_ids.insert(imp_nid.clone());
                nodes.push(Node {
                    id: imp_nid.clone(),
                    label: mod_name,
                    file_type: "code".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    metadata: None,
                });
                edges.push(Edge {
                    source: scope_nid.to_string(),
                    target: imp_nid,
                    relation: "imports".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: Some("use".to_string()),
                    confidence_score: None,
                });
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_fortran(ctx, cur.node(), source, scope_nid);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` edges within a Fortran scope's byte range.
///
/// Recurses through the AST, only visiting nodes that overlap the `[body_start, body_end)` range
/// of the enclosing scope. Emits `calls` edges for `call_expression` nodes that match a known
/// NID. Mirrors Python `_walk_calls_fortran`.
/// Shared state threaded through every [`walk_calls_fortran`] recursion.
struct FortranCallCtx<'a, 'h> {
    str_path: &'a str,
    stem: &'a str,
    stmt_headers: &'a [&'h str],
    edges: &'a mut Vec<Edge>,
}

fn walk_calls_fortran(
    ctx: &mut FortranCallCtx<'_, '_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    scope_nid: &str,
    body_start: usize,
    body_end: usize,
) {
    let str_path = ctx.str_path;
    let stem = ctx.stem;
    let stmt_headers = ctx.stmt_headers;
    let edges = &mut *ctx.edges;
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if matches!(
        node.kind(),
        "subroutine" | "function" | "module" | "program" | "internal_procedures"
    ) {
        return;
    }
    if node.kind() == "subroutine_call" {
        let name_node = {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                let mut f = None;
                loop {
                    if cur.node().kind() == "identifier" {
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
        if let Some(nn) = name_node {
            let callee = read_text(nn, source).to_lowercase();
            let target_nid = make_id(&[stem, &callee]);
            let line = node.start_position().row + 1;
            edges.push(Edge {
                source: scope_nid.to_string(),
                target: target_nid,
                relation: "calls".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: Some("call".to_string()),
                confidence_score: None,
            });
        }
        return;
    }
    if stmt_headers.contains(&node.kind()) {
        return;
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_fortran(ctx, cur.node(), source, scope_nid, body_start, body_end);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
