//! Fortran extractor — custom walk over tree-sitter-fortran AST.

// Tree-sitter row numbers are source line indices; no file has 2^32 lines.
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::generic::walk::{first_child_kind, named_children};
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

/// Resolve *path* to an absolute path for safe use as a `cpp` argument (F5).
///
/// A corpus file is attacker-named and `cpp` does not accept a `--`
/// end-of-options terminator, so a file named like `-I/etc/passwd.F90` would
/// otherwise be parsed by `cpp` as an option. An absolute path always begins
/// with `/`, so it can never look like an option. Mirrors Python's
/// `path.resolve()`: resolve symlinks where possible, else join the current
/// directory. Returns an absolute path in all normal cases; only if the current
/// working directory cannot be read does it fall back to a `./`-prefixed
/// relative path (still safe — it cannot be parsed as a `cpp` option).
#[must_use]
pub fn resolve_cpp_path(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Ok(cwd) = std::env::current_dir() {
        return cwd.join(path);
    }
    // Last-resort fallback (cwd unavailable): prefix `./` so an attacker-named
    // relative path like `-I/etc/x.F90` still can't be parsed as a cpp option.
    Path::new(".").join(path)
}

/// Run the C preprocessor on a Fortran file to expand macros and `#include` directives.
///
/// Used for free-form Fortran files with `.F` / `.F90` / etc. extensions that use the
/// C preprocessor. Falls back to reading the raw bytes if `cpp` is unavailable or fails.
fn cpp_preprocess(path: &Path) -> Vec<u8> {
    // Security: pass -nostdinc -I /dev/null to prevent file exfiltration, and an
    // absolute path (resolve_cpp_path) so an attacker-named file can't be parsed
    // as a cpp option (F5).
    let result = std::process::Command::new("cpp")
        .args(["-w", "-P", "-nostdinc", "-I", "/dev/null"])
        .arg(resolve_cpp_path(path))
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

    crate::forward_refs::reconcile_forward_refs(&mut nodes, &mut edges);
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

/// Mutable graph state for the Fortran signature-reference pass, reborrowed
/// from the structural-walk locals at each call site.
struct FortranRefCtx<'a> {
    source: &'a [u8],
    stem: &'a str,
    str_path: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
}

impl FortranRefCtx<'_> {
    fn ensure_named_node(&mut self, name: &str, line: usize) -> String {
        let nid1 = make_id(&[self.stem, name]);
        if self.seen_ids.contains(&nid1) {
            return nid1;
        }
        let nid2 = make_id1(name);
        if self.seen_ids.insert(nid2.clone()) {
            self.nodes.push(Node {
                id: nid2.clone(),
                label: name.to_string(),
                file_type: "code".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
            });
        }
        nid2
    }

    fn push_ref(&mut self, src: &str, tgt: &str, context: &str, line: usize) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: "references".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: Some(context.to_string()),
            confidence_score: None,
        });
    }
}

/// Emit `references[parameter_type]` / `references[return_type]` edges for a
/// subroutine/function based on its `derived_type` variable declarations.
/// Mirrors Python `emit_signature_refs`.
fn emit_fortran_signature_refs(
    rc: &mut FortranRefCtx<'_>,
    scope_node: tree_sitter::Node<'_>,
    fn_nid: &str,
    is_function: bool,
) {
    let stmt_type = if is_function {
        "function_statement"
    } else {
        "subroutine_statement"
    };
    let Some(stmt) = first_child_kind(scope_node, stmt_type) else {
        return;
    };

    let mut param_names: HashSet<String> = HashSet::new();
    if let Some(params_node) = first_child_kind(stmt, "parameters") {
        for c in named_children(params_node) {
            if c.kind() == "identifier" {
                param_names.insert(read_text(c, rc.source).to_lowercase());
            }
        }
    }

    let mut result_name: Option<String> = None;
    if is_function {
        if let Some(result_node) = first_child_kind(stmt, "function_result") {
            if let Some(res_id) = first_child_kind(result_node, "identifier") {
                result_name = Some(read_text(res_id, rc.source).to_lowercase());
            }
        } else {
            // Implicit result variable: same name as the function.
            result_name = fortran_name(stmt, rc.source);
        }
    }

    for child in named_children(scope_node) {
        if child.kind() != "variable_declaration" {
            continue;
        }
        let Some(derived) = first_child_kind(child, "derived_type") else {
            continue;
        };
        let Some(type_name_node) = first_child_kind(derived, "type_name") else {
            continue;
        };
        let type_name = read_text(type_name_node, rc.source).to_lowercase();
        for var in named_children(child) {
            if var.kind() != "identifier" {
                continue;
            }
            let var_name = read_text(var, rc.source).to_lowercase();
            let var_line = var.start_position().row + 1;
            if param_names.contains(&var_name) {
                let tgt = rc.ensure_named_node(&type_name, var_line);
                if tgt != fn_nid {
                    rc.push_ref(fn_nid, &tgt, "parameter_type", var_line);
                }
            } else if is_function && result_name.as_deref() == Some(var_name.as_str()) {
                let tgt = rc.ensure_named_node(&type_name, var_line);
                if tgt != fn_nid {
                    rc.push_ref(fn_nid, &tgt, "return_type", var_line);
                }
            }
        }
    }
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
                    external: false,
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
                    external: false,
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
                    external: false,
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
                let mut rc = FortranRefCtx {
                    source,
                    stem,
                    str_path,
                    nodes: &mut *nodes,
                    edges: &mut *edges,
                    seen_ids: &mut *seen_ids,
                };
                emit_fortran_signature_refs(&mut rc, node, &nid, false);
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
                    external: false,
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
                let mut rc = FortranRefCtx {
                    source,
                    stem,
                    str_path,
                    nodes: &mut *nodes,
                    edges: &mut *edges,
                    seen_ids: &mut *seen_ids,
                };
                emit_fortran_signature_refs(&mut rc, node, &nid, true);
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
        "derived_type_definition" => {
            if let Some(stmt) = first_child_kind(node, "derived_type_statement")
                && let Some(name_node) = first_child_kind(stmt, "type_name")
            {
                let type_name = read_text(name_node, source).to_lowercase();
                let type_nid = make_id(&[stem, &type_name]);
                let line = node.start_position().row + 1;
                if seen_ids.insert(type_nid.clone()) {
                    nodes.push(Node {
                        id: type_nid.clone(),
                        label: type_name,
                        file_type: "code".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    external: false,
                    source: scope_nid.to_string(),
                    target: type_nid,
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
                    external: false,
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
                external: false,
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
