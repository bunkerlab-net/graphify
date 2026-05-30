//! Go extractor — custom walk over tree-sitter-go AST.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Go's predeclared type identifiers — never emitted as semantic type references.
const GO_PREDECLARED_TYPES: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
    "comparable",
];

/// Walk a Go type expression, appending `(name, is_generic_arg)` tuples for each
/// user-defined type referenced. Predeclared types are skipped. Mirrors Python
/// `_go_collect_type_refs`.
fn go_collect_type_refs(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    match node.kind() {
        "type_identifier" => {
            let text = read_text(node, source);
            if !text.is_empty() && !GO_PREDECLARED_TYPES.contains(&text) {
                out.push((text.to_string(), generic));
            }
        }
        "qualified_type" => {
            let full = read_text(node, source);
            let text = full.rsplit('.').next().unwrap_or(full);
            if !text.is_empty() && !GO_PREDECLARED_TYPES.contains(&text) {
                out.push((text.to_string(), generic));
            }
        }
        "generic_type" => {
            if let Some(type_field) = node.child_by_field_name("type") {
                go_collect_type_refs(type_field, source, generic, out);
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        let mut acur = cur.node().walk();
                        if acur.goto_first_child() {
                            loop {
                                if acur.node().is_named() {
                                    go_collect_type_refs(acur.node(), source, true, out);
                                }
                                if !acur.goto_next_sibling() {
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
        "pointer_type" | "slice_type" | "array_type" | "map_type" | "channel_type"
        | "parenthesized_type" => {
            recurse_named_children(node, source, generic, out);
        }
        _ if node.is_named() => recurse_named_children(node, source, generic, out),
        _ => {}
    }
}

/// Recurse `go_collect_type_refs` over every named child of `node`.
fn recurse_named_children(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().is_named() {
                go_collect_type_refs(cur.node(), source, generic, out);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Mutable graph state for the Go semantic-reference passes. Constructed by
/// reborrowing the structural-walk locals at each call site so these passes
/// never need to thread the full [`GoWalkCtx`].
struct GoRefCtx<'a> {
    source: &'a [u8],
    pkg_scope: &'a str,
    str_path: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
}

impl GoRefCtx<'_> {
    /// Return the NID for a named type, creating a bare placeholder node when no
    /// package-qualified node already exists. Mirrors Go's `ensure_named_node`.
    fn ensure_named_node(&mut self, name: &str, line: usize) -> String {
        let nid1 = make_id(&[self.pkg_scope, name]);
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

    /// Push a `references` edge from `src` to `tgt` with the given context.
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

    /// Push a plain `embeds` edge from `src` to `tgt` (Go struct/interface embedding).
    fn push_embeds(&mut self, src: &str, tgt: &str, line: usize) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: "embeds".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    }
}

/// Emit `references` edges for a function/method's parameter and result types.
///
/// Mirrors Python `emit_go_method_refs`: direct param types use the
/// `parameter_type` context, result types use `return_type`, and any generic
/// arguments use `generic_arg`.
fn emit_go_method_refs(
    rc: &mut GoRefCtx<'_>,
    func_node: tree_sitter::Node<'_>,
    func_nid: &str,
    line: usize,
) {
    if let Some(params) = func_node.child_by_field_name("parameters") {
        let mut cur = params.walk();
        if cur.goto_first_child() {
            loop {
                let p = cur.node();
                if p.kind() == "parameter_declaration"
                    && let Some(type_node) = p.child_by_field_name("type")
                {
                    let mut refs: Vec<(String, bool)> = Vec::new();
                    go_collect_type_refs(type_node, rc.source, false, &mut refs);
                    for (ref_name, is_generic) in refs {
                        let ctx = if is_generic {
                            "generic_arg"
                        } else {
                            "parameter_type"
                        };
                        let tgt = rc.ensure_named_node(&ref_name, line);
                        if tgt != func_nid {
                            rc.push_ref(func_nid, &tgt, ctx, line);
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    let Some(result) = func_node.child_by_field_name("result") else {
        return;
    };
    if result.kind() == "parameter_list" {
        let mut cur = result.walk();
        if cur.goto_first_child() {
            loop {
                let p = cur.node();
                if p.kind() == "parameter_declaration" {
                    let type_node = p.child_by_field_name("type").or_else(|| {
                        let mut c = p.walk();
                        if c.goto_first_child() {
                            loop {
                                if c.node().is_named() {
                                    return Some(c.node());
                                }
                                if !c.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                        None
                    });
                    if let Some(tn) = type_node {
                        emit_go_result_refs(rc, tn, func_nid, line);
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    } else {
        emit_go_result_refs(rc, result, func_nid, line);
    }
}

/// Emit `return_type` / `generic_arg` references from a single result type node.
fn emit_go_result_refs(
    rc: &mut GoRefCtx<'_>,
    type_node: tree_sitter::Node<'_>,
    func_nid: &str,
    line: usize,
) {
    let mut refs: Vec<(String, bool)> = Vec::new();
    go_collect_type_refs(type_node, rc.source, false, &mut refs);
    for (ref_name, is_generic) in refs {
        let ctx = if is_generic {
            "generic_arg"
        } else {
            "return_type"
        };
        let tgt = rc.ensure_named_node(&ref_name, line);
        if tgt != func_nid {
            rc.push_ref(func_nid, &tgt, ctx, line);
        }
    }
}

/// Emit `embeds` / `references[field]` edges for a `type_spec`'s struct fields,
/// and `embeds` / `references[generic_arg]` edges for interface embedding.
///
/// A struct field with no name and a direct (non-generic) type is an embedded
/// field → `embeds`; named fields and generic args become `references`. Mirrors
/// the struct/interface body handling added to Python `extract_go`.
fn emit_go_type_body_refs(rc: &mut GoRefCtx<'_>, type_spec: tree_sitter::Node<'_>, type_nid: &str) {
    let mut type_body: Option<tree_sitter::Node<'_>> = None;
    let mut cur = type_spec.walk();
    if cur.goto_first_child() {
        loop {
            if matches!(cur.node().kind(), "struct_type" | "interface_type") {
                type_body = Some(cur.node());
                break;
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    let Some(type_body) = type_body else {
        return;
    };

    if type_body.kind() == "struct_type" {
        let mut fdl_cur = type_body.walk();
        if !fdl_cur.goto_first_child() {
            return;
        }
        loop {
            if fdl_cur.node().kind() == "field_declaration_list" {
                let mut fcur = fdl_cur.node().walk();
                if fcur.goto_first_child() {
                    loop {
                        if fcur.node().kind() == "field_declaration" {
                            emit_go_struct_field_refs(rc, fcur.node(), type_nid);
                        }
                        if !fcur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if !fdl_cur.goto_next_sibling() {
                break;
            }
        }
    } else {
        // interface_type — embedded interfaces appear as `type_elem`.
        let mut ecur = type_body.walk();
        if !ecur.goto_first_child() {
            return;
        }
        loop {
            if ecur.node().kind() == "type_elem" {
                let line = ecur.node().start_position().row + 1;
                let mut refs: Vec<(String, bool)> = Vec::new();
                let mut scur = ecur.node().walk();
                if scur.goto_first_child() {
                    loop {
                        if scur.node().is_named() {
                            go_collect_type_refs(scur.node(), rc.source, false, &mut refs);
                        }
                        if !scur.goto_next_sibling() {
                            break;
                        }
                    }
                }
                for (ref_name, is_generic) in refs {
                    let tgt = rc.ensure_named_node(&ref_name, line);
                    if tgt == type_nid {
                        continue;
                    }
                    if is_generic {
                        rc.push_ref(type_nid, &tgt, "generic_arg", line);
                    } else {
                        rc.push_embeds(type_nid, &tgt, line);
                    }
                }
            }
            if !ecur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Emit edges for a single Go struct `field_declaration`.
fn emit_go_struct_field_refs(rc: &mut GoRefCtx<'_>, field: tree_sitter::Node<'_>, type_nid: &str) {
    let line = field.start_position().row + 1;
    let mut has_name = false;
    let mut fallback_type: Option<tree_sitter::Node<'_>> = None;
    let mut fcur = field.walk();
    if fcur.goto_first_child() {
        loop {
            let fc = fcur.node();
            if fc.kind() == "field_identifier" {
                has_name = true;
            } else if fallback_type.is_none() && fc.is_named() {
                fallback_type = Some(fc);
            }
            if !fcur.goto_next_sibling() {
                break;
            }
        }
    }
    let Some(type_node) = field.child_by_field_name("type").or(fallback_type) else {
        return;
    };
    let mut refs: Vec<(String, bool)> = Vec::new();
    go_collect_type_refs(type_node, rc.source, false, &mut refs);
    for (ref_name, is_generic) in refs {
        let tgt = rc.ensure_named_node(&ref_name, line);
        if tgt == type_nid {
            continue;
        }
        if !has_name && !is_generic {
            rc.push_embeds(type_nid, &tgt, line);
        } else {
            let ctx = if is_generic { "generic_arg" } else { "field" };
            rc.push_ref(type_nid, &tgt, ctx, line);
        }
    }
}

/// Extract functions, methods, type declarations, and imports from a `.go` file.
#[must_use]
pub fn extract_go(path: &Path) -> FileResult {
    let Some((source, tree)) = parse_go_source(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("parse failed".to_string()),
        };
    };

    let stem = file_stem(path);
    let pkg_scope = derive_pkg_scope(path, &stem);
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
    let mut go_imported_pkgs: HashSet<String> = HashSet::new();

    {
        let mut walk_ctx = GoWalkCtx {
            str_path: &str_path,
            stem: &stem,
            pkg_scope: &pkg_scope,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
            go_imported_pkgs: &mut go_imported_pkgs,
        };
        walk_go(&mut walk_ctx, tree.root_node(), &source);
    }

    let label_to_nid = build_go_label_map(&nodes);
    let raw_calls = resolve_go_function_calls(GoResolveArgs {
        tree: &tree,
        source: &source,
        str_path: &str_path,
        function_bodies: &function_bodies,
        label_to_nid: &label_to_nid,
        go_imported_pkgs: &go_imported_pkgs,
        edges: &mut edges,
    });
    let clean_edges = filter_dangling_edges(edges, &seen_ids);

    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}

/// Read the file and parse with tree-sitter-go. `None` on any I/O or parse error.
fn parse_go_source(path: &Path) -> Option<(Vec<u8>, tree_sitter::Tree)> {
    let source = std::fs::read(path).ok()?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    let tree = parser.parse(&source, None)?;
    Some((source, tree))
}

/// Use the directory name as package scope so methods on the same type share a
/// canonical type node across files in the same package.
fn derive_pkg_scope(path: &Path, fallback_stem: &str) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_stem)
        .to_string()
}

/// Build a `normalised_label → nid` map for intra-file call resolution.
fn build_go_label_map(nodes: &[Node]) -> HashMap<String, String> {
    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }
    label_to_nid
}

/// Bundle of shared inputs for [`resolve_go_function_calls`].
struct GoResolveArgs<'a> {
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    str_path: &'a str,
    function_bodies: &'a [(String, usize, usize)],
    label_to_nid: &'a HashMap<String, String>,
    go_imported_pkgs: &'a HashSet<String>,
    edges: &'a mut Vec<Edge>,
}

/// Walk each function body to emit call edges and `RawCall` entries.
fn resolve_go_function_calls(args: GoResolveArgs<'_>) -> Vec<RawCall> {
    let GoResolveArgs {
        tree,
        source,
        str_path,
        function_bodies,
        label_to_nid,
        go_imported_pkgs,
        edges,
    } = args;
    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    {
        let mut call_ctx = GoCallCtx {
            str_path,
            label_to_nid,
            go_imported_pkgs,
            edges,
            seen_call_pairs: &mut seen_call_pairs,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_start, body_end) in function_bodies {
            walk_calls_go(
                &mut call_ctx,
                tree.root_node(),
                source,
                caller_nid,
                *body_start,
                *body_end,
            );
        }
    }
    raw_calls
}

/// Drop edges whose endpoints aren't in `valid_ids` (except for `imports` edges).
fn filter_dangling_edges(edges: Vec<Edge>, valid_ids: &HashSet<String>) -> Vec<Edge> {
    edges
        .into_iter()
        .filter(|e| {
            valid_ids.contains(&e.source)
                && (valid_ids.contains(&e.target)
                    || matches!(e.relation.as_str(), "imports" | "imports_from"))
        })
        .collect()
}

/// Recursively walk a Go AST emitting nodes and edges for functions, methods, and type declarations.
///
/// Handles `function_declaration`, `method_declaration`, `type_declaration`, and `import_declaration`
/// nodes. Descends into all child nodes. Mirrors Python `_walk_go`.
/// Shared state threaded through every [`walk_go`] recursion.
struct GoWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    pkg_scope: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    function_bodies: &'a mut Vec<(String, usize, usize)>,
    go_imported_pkgs: &'a mut HashSet<String>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Go's AST node kinds
fn walk_go(ctx: &mut GoWalkCtx<'_>, node: tree_sitter::Node<'_>, source: &[u8]) {
    let str_path = ctx.str_path;
    let stem = ctx.stem;
    let pkg_scope = ctx.pkg_scope;
    let file_nid = ctx.file_nid;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let function_bodies = &mut *ctx.function_bodies;
    let go_imported_pkgs = &mut *ctx.go_imported_pkgs;
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
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    external: false,
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
                let mut rc = GoRefCtx {
                    source,
                    pkg_scope,
                    str_path,
                    nodes: &mut *nodes,
                    edges: &mut *edges,
                    seen_ids: &mut *seen_ids,
                };
                emit_go_method_refs(&mut rc, node, &func_nid, line);
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
                            metadata: None,
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
                            metadata: None,
                        });
                    }
                    edges.push(Edge {
                        external: false,
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
                            metadata: None,
                        });
                    }
                    edges.push(Edge {
                        external: false,
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
                let mut rc = GoRefCtx {
                    source,
                    pkg_scope,
                    str_path,
                    nodes: &mut *nodes,
                    edges: &mut *edges,
                    seen_ids: &mut *seen_ids,
                };
                emit_go_method_refs(&mut rc, node, &method_nid, line);
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
                                metadata: None,
                            });
                        }
                        edges.push(Edge {
                            external: false,
                            source: file_nid.to_string(),
                            target: type_nid.clone(),
                            relation: "contains".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        // Struct field embeds/references and interface embedding.
                        let mut rc = GoRefCtx {
                            source,
                            pkg_scope,
                            str_path,
                            nodes: &mut *nodes,
                            edges: &mut *edges,
                            seen_ids: &mut *seen_ids,
                        };
                        emit_go_type_body_refs(&mut rc, child, &type_nid);
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
                    walk_go(ctx, cur.node(), source);
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
            external: false,
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
/// Shared state threaded through every [`walk_calls_go`] recursion.
struct GoCallCtx<'a> {
    str_path: &'a str,
    label_to_nid: &'a HashMap<String, String>,
    go_imported_pkgs: &'a HashSet<String>,
    edges: &'a mut Vec<Edge>,
    seen_call_pairs: &'a mut HashSet<(String, String)>,
    raw_calls: &'a mut Vec<RawCall>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Go's call-site AST shapes
fn walk_calls_go(
    ctx: &mut GoCallCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
) {
    let str_path = ctx.str_path;
    let label_to_nid = ctx.label_to_nid;
    let go_imported_pkgs = ctx.go_imported_pkgs;
    let edges = &mut *ctx.edges;
    let seen_call_pairs = &mut *ctx.seen_call_pairs;
    let raw_calls = &mut *ctx.raw_calls;
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
                // Built-in suppression applies only to unqualified identifier
                // calls; a selector call (`obj.len()`, `pkg.len()`) names a method
                // or package function, not the language built-in, so it must not
                // be filtered.
                let is_unqualified = func_node.kind() == "identifier";
                if let Some(cn) = callee_name {
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
                    } else if !(is_unqualified && crate::builtins::is_language_builtin_global(&cn))
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
            // Recurse into arguments
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_calls_go(ctx, cur.node(), source, caller_nid, body_start, body_end);
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
                    walk_calls_go(ctx, cur.node(), source, caller_nid, body_start, body_end);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}
