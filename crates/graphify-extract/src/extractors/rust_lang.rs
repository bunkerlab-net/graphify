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
                emit_rust_param_return_refs(ctx, node, &func_nid, line, source);
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
                    target: item_nid.clone(),
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                if t == "trait_item" {
                    emit_rust_trait_bounds(ctx, node, &item_nid, line, source);
                }
                if t == "struct_item" {
                    emit_rust_struct_fields(ctx, node, &item_nid, source);
                }
            }
        }
        "impl_item" => {
            let line = node.start_position().row + 1;
            let mut impl_nid: Option<String> = None;
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = read_text(type_node, source).trim().to_string();
                let nid = make_id(&[ctx.stem, &type_name]);
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
            if let (Some(trait_node), Some(inid)) =
                (node.child_by_field_name("trait"), impl_nid.clone())
            {
                emit_rust_impl_trait(ctx, trait_node, &inid, line, source);
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

/// Walk a Rust type expression, appending `(name, is_generic_arg)` tuples for
/// each user-defined type referenced. Primitive types are skipped. Mirrors
/// Python `_rust_collect_type_refs`.
fn rust_collect_type_refs(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    match node.kind() {
        "primitive_type" => {}
        "type_identifier" => {
            let text = read_text(node, source);
            if !text.is_empty() {
                out.push((text.to_string(), generic));
            }
        }
        "scoped_type_identifier" => {
            let full = read_text(node, source);
            let text = full.rsplit("::").next().unwrap_or(full);
            if !text.is_empty() {
                out.push((text.to_string(), generic));
            }
        }
        "generic_type" => {
            let name_node = node.child_by_field_name("type").or_else(|| {
                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        if matches!(
                            c.node().kind(),
                            "type_identifier" | "scoped_type_identifier"
                        ) {
                            return Some(c.node());
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                None
            });
            if let Some(nn) = name_node {
                let full = read_text(nn, source);
                let text = full.rsplit("::").next().unwrap_or(full);
                if !text.is_empty() {
                    out.push((text.to_string(), generic));
                }
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        let mut acur = cur.node().walk();
                        if acur.goto_first_child() {
                            loop {
                                if acur.node().is_named() {
                                    rust_collect_type_refs(acur.node(), source, true, out);
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
        "reference_type" | "pointer_type" | "array_type" | "tuple_type" | "slice_type" => {
            rust_recurse_named(node, source, generic, out);
        }
        _ if node.is_named() => rust_recurse_named(node, source, generic, out),
        _ => {}
    }
}

/// Recurse `rust_collect_type_refs` over every named child of `node`.
fn rust_recurse_named(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, bool)>,
) {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().is_named() {
                rust_collect_type_refs(cur.node(), source, generic, out);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

impl RustWalkCtx<'_> {
    /// Return the NID for a named type, creating a bare placeholder node when no
    /// file-qualified node already exists. Mirrors Rust's `ensure_named_node`.
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

    /// Push a `references` edge with the given context.
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

    /// Push a plain relation edge (e.g. `inherits`, `implements`).
    fn push_rel(&mut self, src: &str, tgt: &str, relation: &str, line: usize) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    }
}

/// Emit `parameter_type` / `return_type` / `generic_arg` references for a `fn`.
fn emit_rust_param_return_refs(
    ctx: &mut RustWalkCtx<'_>,
    func_node: tree_sitter::Node<'_>,
    func_nid: &str,
    line: usize,
    source: &[u8],
) {
    if let Some(params) = func_node.child_by_field_name("parameters") {
        let mut cur = params.walk();
        if cur.goto_first_child() {
            loop {
                let p = cur.node();
                if p.kind() == "parameter"
                    && let Some(type_node) = p.child_by_field_name("type")
                {
                    let mut refs: Vec<(String, bool)> = Vec::new();
                    rust_collect_type_refs(type_node, source, false, &mut refs);
                    for (ref_name, is_generic) in refs {
                        let context = if is_generic {
                            "generic_arg"
                        } else {
                            "parameter_type"
                        };
                        let tgt = ctx.ensure_named_node(&ref_name, line);
                        if tgt != func_nid {
                            ctx.push_ref(func_nid, &tgt, context, line);
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    if let Some(return_type) = func_node.child_by_field_name("return_type") {
        let mut refs: Vec<(String, bool)> = Vec::new();
        rust_collect_type_refs(return_type, source, false, &mut refs);
        for (ref_name, is_generic) in refs {
            let context = if is_generic {
                "generic_arg"
            } else {
                "return_type"
            };
            let tgt = ctx.ensure_named_node(&ref_name, line);
            if tgt != func_nid {
                ctx.push_ref(func_nid, &tgt, context, line);
            }
        }
    }
}

/// Emit `inherits` (first supertrait) / `references[generic_arg]` edges from a
/// `trait_item`'s `trait_bounds`.
fn emit_rust_trait_bounds(
    ctx: &mut RustWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    item_nid: &str,
    line: usize,
    source: &[u8],
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        if cur.node().kind() == "trait_bounds" {
            let mut bcur = cur.node().walk();
            if bcur.goto_first_child() {
                loop {
                    if bcur.node().is_named() {
                        let mut refs: Vec<(String, bool)> = Vec::new();
                        rust_collect_type_refs(bcur.node(), source, false, &mut refs);
                        for (idx, (ref_name, _is_generic)) in refs.into_iter().enumerate() {
                            let tgt = ctx.ensure_named_node(&ref_name, line);
                            if tgt == item_nid {
                                continue;
                            }
                            if idx == 0 {
                                ctx.push_rel(item_nid, &tgt, "inherits", line);
                            } else {
                                ctx.push_ref(item_nid, &tgt, "generic_arg", line);
                            }
                        }
                    }
                    if !bcur.goto_next_sibling() {
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

/// Emit `references[field]` / `references[generic_arg]` edges from a
/// `struct_item`'s field declarations.
fn emit_rust_struct_fields(
    ctx: &mut RustWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    item_nid: &str,
    source: &[u8],
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        if cur.node().kind() == "field_declaration_list" {
            let mut fcur = cur.node().walk();
            if fcur.goto_first_child() {
                loop {
                    if fcur.node().kind() == "field_declaration" {
                        emit_rust_struct_field(ctx, fcur.node(), item_nid, source);
                    }
                    if !fcur.goto_next_sibling() {
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

/// Emit references for a single Rust struct `field_declaration`.
fn emit_rust_struct_field(
    ctx: &mut RustWalkCtx<'_>,
    field: tree_sitter::Node<'_>,
    item_nid: &str,
    source: &[u8],
) {
    let line = field.start_position().row + 1;
    let type_node = field.child_by_field_name("type").or_else(|| {
        let mut c = field.walk();
        if c.goto_first_child() {
            loop {
                if matches!(
                    c.node().kind(),
                    "type_identifier"
                        | "generic_type"
                        | "scoped_type_identifier"
                        | "reference_type"
                        | "primitive_type"
                ) {
                    return Some(c.node());
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    });
    let Some(type_node) = type_node else {
        return;
    };
    let mut refs: Vec<(String, bool)> = Vec::new();
    rust_collect_type_refs(type_node, source, false, &mut refs);
    for (ref_name, is_generic) in refs {
        let context = if is_generic { "generic_arg" } else { "field" };
        let tgt = ctx.ensure_named_node(&ref_name, line);
        if tgt != item_nid {
            ctx.push_ref(item_nid, &tgt, context, line);
        }
    }
}

/// Emit `implements` (first trait) / `references[generic_arg]` edges from an
/// `impl Trait for Type` block's `trait` node.
fn emit_rust_impl_trait(
    ctx: &mut RustWalkCtx<'_>,
    trait_node: tree_sitter::Node<'_>,
    impl_nid: &str,
    line: usize,
    source: &[u8],
) {
    let mut refs: Vec<(String, bool)> = Vec::new();
    rust_collect_type_refs(trait_node, source, false, &mut refs);
    for (idx, (ref_name, _is_generic)) in refs.into_iter().enumerate() {
        let tgt = ctx.ensure_named_node(&ref_name, line);
        if tgt == impl_nid {
            continue;
        }
        if idx == 0 {
            ctx.push_rel(impl_nid, &tgt, "implements", line);
        } else {
            ctx.push_ref(impl_nid, &tgt, "generic_arg", line);
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
