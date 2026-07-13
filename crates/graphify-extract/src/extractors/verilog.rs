//! Verilog/SystemVerilog extractor — custom walk over tree-sitter-verilog AST.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// First `simple_identifier` under `node` in pre-order, or `None`.
///
/// tree-sitter-verilog 1.0.3 nests declaration names a few levels deep instead
/// of exposing a `name` field, so the older `child_by_field_name("name")`
/// approach returned `None` for every declaration. Scope the search to the
/// right child node (e.g. `function_identifier`) or this returns the
/// return-type instead of the name.
fn sv_first_identifier(node: Option<tree_sitter::Node<'_>>, source: &[u8]) -> Option<String> {
    let node = node?;
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "simple_identifier" {
                return Some(read_text(child, source).to_string());
            }
            if let Some(found) = sv_first_identifier(Some(child), source) {
                return Some(found);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// First direct child of `node` whose kind is `type_name`, or `None`.
fn sv_child<'a>(
    node: Option<tree_sitter::Node<'a>>,
    type_name: &str,
) -> Option<tree_sitter::Node<'a>> {
    let node = node?;
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().kind() == type_name {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Extract modules, functions, tasks, package imports, and instantiations from `.v`/`.sv` files.
#[must_use]
pub fn extract_verilog(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_verilog::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set verilog language".to_string()),
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
        node_type: None,
    });

    let root = tree.root_node();
    {
        let mut walk_ctx = VerilogWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
        };
        walk_verilog(&mut walk_ctx, root, &source, None);
    }

    // SystemVerilog class semantics (inherits/implements edges, field /
    // parameter / return-type references) are recovered with a regex pass over
    // the source text — the 1.0.3 grammar does not expose usable class bodies.
    let raw = String::from_utf8_lossy(&source).into_owned();
    augment_systemverilog_semantics(
        &raw,
        &stem,
        &str_path,
        &file_nid,
        &mut nodes,
        &mut edges,
        &mut seen_ids,
    );

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Recursively walk a Verilog / `SystemVerilog` AST emitting nodes for modules, functions, and tasks.
///
/// Handles `module_declaration`, `function_declaration`, `task_declaration`, and
/// `module_instantiation` (as `uses` edges). Mirrors Python `_walk_verilog`.
/// Shared state threaded through every [`walk_verilog`] recursion.
struct VerilogWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
}

/// Push a node if its id has not been seen, recording the id.
fn push_node_once(ctx: &mut VerilogWalkCtx<'_>, nid: &str, label: &str, line: usize) {
    if ctx.seen_ids.insert(nid.to_string()) {
        ctx.nodes.push(Node {
            id: nid.to_string(),
            label: label.to_string(),
            file_type: "code".to_string(),
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            metadata: None,
            origin_file: None,
            node_type: None,
        });
    }
}

/// Push an edge with the standard Verilog extraction defaults.
fn push_edge(ctx: &mut VerilogWalkCtx<'_>, src: &str, tgt: &str, relation: &str, line: usize) {
    ctx.edges.push(Edge {
        external: false,
        source: src.to_string(),
        target: tgt.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: ctx.str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: None,
        confidence_score: None,
        deferred: false,
        metadata: None,
    });
}

/// Recurse into every child of `node`, carrying the current module context.
fn walk_children(
    ctx: &mut VerilogWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    module_nid: Option<&str>,
) {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_verilog(ctx, cur.node(), source, module_nid);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

fn walk_verilog(
    ctx: &mut VerilogWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    module_nid: Option<&str>,
) {
    let t = node.kind();

    // SystemVerilog class bodies are handled by `augment_systemverilog_semantics`
    // (regex over source text). Skip their subtrees so in-class methods are not
    // double-emitted here — and with the wrong, return-type-derived name.
    if matches!(t, "class_declaration" | "interface_class_declaration") {
        return;
    }

    if t == "module_declaration" {
        if let Some(mod_name) = sv_first_identifier(sv_child(Some(node), "module_header"), source) {
            let line = node.start_position().row + 1;
            let nid = make_id(&[ctx.stem, &mod_name]);
            let file_nid = ctx.file_nid.to_string();
            push_node_once(ctx, &nid, &mod_name, line);
            push_edge(ctx, &file_nid, &nid, "defines", line);
            walk_children(ctx, node, source, Some(&nid));
            return;
        }
        // No resolvable name — fall through to the generic recursion below.
    } else if t == "function_declaration" {
        // `function_prototype` only appears inside class/interface-class bodies
        // (skipped above) and nests its name differently; it is intentionally
        // not handled here.
        let fn_body = sv_child(Some(node), "function_body_declaration");
        if let Some(func_name) =
            sv_first_identifier(sv_child(fn_body, "function_identifier"), source)
        {
            let line = node.start_position().row + 1;
            let parent = module_nid.unwrap_or(ctx.file_nid).to_string();
            let nid = make_id(&[&parent, &func_name]);
            push_node_once(ctx, &nid, &format!("{func_name}()"), line);
            push_edge(ctx, &parent, &nid, "contains", line);
        }
    } else if t == "task_declaration" {
        let tk_body = sv_child(Some(node), "task_body_declaration");
        if let Some(task_name) = sv_first_identifier(sv_child(tk_body, "task_identifier"), source) {
            let line = node.start_position().row + 1;
            let parent = module_nid.unwrap_or(ctx.file_nid).to_string();
            let nid = make_id(&[&parent, &task_name]);
            push_node_once(ctx, &nid, &task_name, line);
            push_edge(ctx, &parent, &nid, "contains", line);
        }
    } else if t == "package_import_declaration" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "package_import_item" {
                    let pkg_text = read_text(cur.node(), source);
                    let pkg_name = pkg_text.split("::").next().unwrap_or("").trim().to_string();
                    if !pkg_name.is_empty() {
                        let line = node.start_position().row + 1;
                        let tgt_nid = make_id1(&pkg_name);
                        push_node_once(ctx, &tgt_nid, &pkg_name, line);
                        let src = module_nid.unwrap_or(ctx.file_nid).to_string();
                        push_edge(ctx, &src, &tgt_nid, "imports_from", line);
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    } else if matches!(t, "module_instantiation" | "checker_instantiation") {
        // `leaf u_leaf();` parses as `checker_instantiation` in 1.0.3;
        // `module_instantiation` (when it occurs) exposes a `module_type` field.
        // Both reduce to the first identifier under the node — the instantiated
        // type, not the instance name (which appears later).
        if let Some(mnid) = module_nid {
            let inst_type = node
                .child_by_field_name("module_type")
                .map(|tn| read_text(tn, source).trim().to_string())
                .or_else(|| sv_first_identifier(Some(node), source));
            if let Some(inst_type) = inst_type.filter(|s| !s.is_empty()) {
                let line = node.start_position().row + 1;
                let tgt_nid = make_id1(&inst_type);
                push_node_once(ctx, &tgt_nid, &inst_type, line);
                let mnid = mnid.to_string();
                push_edge(ctx, &mnid, &tgt_nid, "instantiates", line);
            }
        }
    }

    walk_children(ctx, node, source, module_nid);
}

// ── SystemVerilog class-level augmentation ──────────────────────────────────
//
// The 1.0.3 grammar does not expose usable class bodies, so class semantics
// (inherits / implements edges and field / parameter / return-type references)
// are recovered with a regex pass over the source text. Mirrors
// `_augment_systemverilog_semantics` and its helpers in graphify-py.

/// Builtin scalar/handle types that are never emitted as referenced types.
const SV_BUILTIN_TYPES: &[&str] = &[
    "bit",
    "logic",
    "reg",
    "wire",
    "int",
    "integer",
    "shortint",
    "longint",
    "byte",
    "time",
    "real",
    "shortreal",
    "void",
    "string",
    "type",
    "event",
    "mailbox",
    "semaphore",
    "process",
    "chandle",
];

/// Keywords that can lead a statement but are not type references.
const SV_NON_TYPE_WORDS: &[&str] = &[
    "return",
    "if",
    "else",
    "for",
    "foreach",
    "while",
    "case",
    "begin",
    "end",
    "function",
    "task",
    "class",
    "endclass",
    "endfunction",
    "endtask",
];

#[allow(clippy::expect_used)] // static literal patterns; compilation cannot fail
static SV_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\b(?:(interface)\s+)?class\s+(\w+)([^;{]*)\s*;(.*?)\bendclass\b")
        .expect("static SV_CLASS_RE regex")
});

#[allow(clippy::expect_used)] // static literal pattern
static SV_TYPE_PARAM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\btype\s+(\w+)").expect("static SV_TYPE_PARAM_RE regex"));

#[allow(clippy::expect_used)] // static literal pattern
static SV_EXTENDS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bextends\s+(\w+)").expect("static SV_EXTENDS_RE regex"));

#[allow(clippy::expect_used)] // static literal pattern
static SV_IMPLEMENTS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bimplements\s+([^;{]+)").expect("static SV_IMPLEMENTS_RE regex")
});

#[allow(clippy::expect_used)] // static literal pattern (function body, DOTALL)
static SV_FUNC_BODY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\bfunction\b.*?\bendfunction\b").expect("static SV_FUNC_BODY_RE regex")
});

#[allow(clippy::expect_used)] // static literal pattern (field declaration, MULTILINE)
static SV_FIELD_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Optional leading class-property qualifiers (rand/local/protected/etc.) are
    // consumed so a qualified field like `rand Config x;` (three tokens) still
    // matches the `<type> <name>;` shape and its type reference is not dropped
    // (297075c). Group 1 remains the type token (after any qualifiers).
    Regex::new(
        r"(?m)^\s*(?:(?:rand|randc|local|protected|static|const|automatic|var)\s+)*([A-Za-z_]\w*(?:\s*#\s*\([^;]+?\))?)\s+\w+\s*;",
    )
    .expect("static SV_FIELD_RE regex")
});

#[allow(clippy::expect_used)] // static literal pattern (one level of balanced parens)
static SV_FUNC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\bfunction\s+([A-Za-z_]\w*(?:\s*#\s*\((?:[^()]|\([^()]*\))*\))?)\s+(\w+)\s*\(((?:[^()]|\([^()]*\))*)\)\s*;",
    )
    .expect("static SV_FUNC_RE regex")
});

#[allow(clippy::expect_used)] // static literal pattern; anchored like Python `.match`
static SV_PARAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:input|output|inout|ref|const\s+ref)?\s*([A-Za-z_]\w*(?:\s*#\s*\((?:[^()]|\([^()]*\))*\))?)\s+\w+",
    )
    .expect("static SV_PARAM_RE regex")
});

#[allow(clippy::expect_used)] // static literal pattern (type head identifier)
static SV_HEAD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z_]\w*)").expect("static SV_HEAD_RE regex"));

#[allow(clippy::expect_used)] // static literal pattern (parameterized type args)
static SV_TYPE_PARAMS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#\s*\(((?:[^()]|\([^()]*\))*)\)").expect("static SV_TYPE_PARAMS_RE regex")
});

#[allow(clippy::expect_used)] // static literal patterns
static SV_BLOCK_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").expect("static SV_BLOCK_COMMENT_RE regex"));

#[allow(clippy::expect_used)] // static literal pattern
static SV_LINE_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"//.*").expect("static SV_LINE_COMMENT_RE regex"));

/// Replace every non-newline character with a space, preserving newlines.
fn blank_keep_newlines(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' { '\n' } else { ' ' })
        .collect()
}

/// Strip `//` and `/* */` comments, blanking them to whitespace so byte offsets
/// and line positions stay aligned with the original source. Diverges from
/// graphify-py (which deletes comment text): preserving length yields exact
/// `source_location` line numbers instead of the reference's approximate ones.
fn sv_strip_comments(text: &str) -> String {
    let no_blocks =
        SV_BLOCK_COMMENT_RE.replace_all(text, |c: &regex::Captures<'_>| blank_keep_newlines(&c[0]));
    SV_LINE_COMMENT_RE
        .replace_all(&no_blocks, |c: &regex::Captures<'_>| {
            blank_keep_newlines(&c[0])
        })
        .into_owned()
}

/// Number of newlines in `text[..offset]` plus one — the 1-based line number.
/// `offset` is a regex match boundary, so it always lands on a `char` boundary.
fn line_for(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

/// Split a comma-separated type/parameter list, honouring one level of nested
/// parentheses so `Foo #(Bar, Baz)` is not split at the inner comma.
fn sv_split_type_list(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = (depth - 1).max(0),
            ',' if depth == 0 => {
                let item = text[start..idx].trim();
                if !item.is_empty() {
                    parts.push(item.to_string());
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    let item = text[start..].trim();
    if !item.is_empty() {
        parts.push(item.to_string());
    }
    parts
}

/// Collect `(referenced_type, is_generic_arg)` pairs from a `SystemVerilog` type
/// expression. `skip` carries the enclosing class's `#(type T = …)` parameters
/// so they are not mistaken for referenced types.
fn sv_collect_type_refs(
    type_text: &str,
    generic: bool,
    skip: &HashSet<String>,
) -> Vec<(String, bool)> {
    let mut refs = Vec::new();
    let text = type_text.trim();
    if text.is_empty() {
        return refs;
    }
    if let Some(head) = SV_HEAD_RE.captures(text) {
        let name = &head[1];
        if !SV_BUILTIN_TYPES.contains(&name)
            && !SV_NON_TYPE_WORDS.contains(&name)
            && !skip.contains(name)
        {
            refs.push((name.to_string(), generic));
        }
    }
    if let Some(params) = SV_TYPE_PARAMS_RE.captures(text) {
        for arg in sv_split_type_list(&params[1]) {
            refs.extend(sv_collect_type_refs(&arg, true, skip));
        }
    }
    refs
}

/// Mutable state for the class-augmentation pass.
struct SvAug<'a> {
    stem: &'a str,
    str_path: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    label_to_nid: HashMap<String, String>,
}

impl SvAug<'_> {
    /// Add a node if its id is new; always (re)map `label → nid`.
    fn add_node(&mut self, nid: &str, label: &str, line: usize) {
        if self.seen_ids.insert(nid.to_string()) {
            self.nodes.push(Node {
                id: nid.to_string(),
                label: label.to_string(),
                file_type: "code".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
                origin_file: None,
                node_type: None,
            });
        }
        self.label_to_nid.insert(label.to_string(), nid.to_string());
    }

    /// Resolve a type label to an existing node id, creating a stub node when
    /// the type is not defined in this file.
    fn ensure_type(&mut self, label: &str, line: usize) -> String {
        if let Some(nid) = self.label_to_nid.get(label) {
            return nid.clone();
        }
        let nid = make_id(&[self.stem, label]);
        self.add_node(&nid, label, line);
        nid
    }

    /// Add an edge whose target is a type label resolved via [`Self::ensure_type`].
    fn add_ref_edge(
        &mut self,
        src: &str,
        target_label: &str,
        relation: &str,
        line: usize,
        context: Option<&str>,
    ) {
        let tgt = self.ensure_type(target_label, line);
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt,
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: context.map(str::to_string),
            confidence_score: None,
            deferred: false,
            metadata: None,
        });
    }

    /// Add an edge to an already-resolved node id.
    fn add_edge(&mut self, src: &str, tgt: &str, relation: &str, line: usize) {
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
            deferred: false,
            metadata: None,
        });
    }
}

/// Recover `SystemVerilog` class semantics from source text and append the
/// resulting nodes/edges. Mirrors `_augment_systemverilog_semantics`.
#[allow(clippy::too_many_lines)] // single cohesive regex pass mirroring the Python reference
fn augment_systemverilog_semantics(
    raw: &str,
    stem: &str,
    str_path: &str,
    file_nid: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
) {
    let label_to_nid: HashMap<String, String> = nodes
        .iter()
        .map(|n| (n.label.clone(), n.id.clone()))
        .collect();
    let mut aug = SvAug {
        stem,
        str_path,
        nodes,
        edges,
        seen_ids,
        label_to_nid,
    };

    let text = sv_strip_comments(raw);
    // Consuming `endclass` (rather than a lookahead) makes each match own its
    // terminator, so back-to-back or malformed classes cannot bleed bodies.
    for cap in SV_CLASS_RE.captures_iter(&text) {
        let Some(whole) = cap.get(0) else { continue };
        let class_name = cap.get(2).map_or("", |g| g.as_str());
        let header = cap.get(3).map_or("", |g| g.as_str());
        let body = cap.get(4).map_or("", |g| g.as_str());
        let line = line_for(&text, whole.start());

        // `#(type T = Payload)` declares `T` as a class type parameter, not a
        // referenced type — collect these to skip below.
        let type_params: HashSet<String> = SV_TYPE_PARAM_RE
            .captures_iter(header)
            .filter_map(|c| c.get(1).map(|g| g.as_str().to_string()))
            .collect();

        let class_nid = make_id(&[stem, class_name]);
        aug.add_node(&class_nid, class_name, line);
        aug.add_edge(file_nid, &class_nid, "defines", line);

        if let Some(ext) = SV_EXTENDS_RE.captures(header) {
            aug.add_ref_edge(&class_nid, &ext[1], "inherits", line, None);
        }
        if let Some(imp) = SV_IMPLEMENTS_RE.captures(header) {
            for iface in sv_split_type_list(&imp[1]) {
                let name = iface.split('#').next().unwrap_or("").trim();
                if !name.is_empty() {
                    aug.add_ref_edge(&class_nid, name, "implements", line, None);
                }
            }
        }

        // Blank function bodies (preserving newline count) so a field-shaped
        // statement inside a method is not mistaken for a class field.
        let body_wo_fn = SV_FUNC_BODY_RE.replace_all(body, |c: &regex::Captures<'_>| {
            "\n".repeat(c[0].matches('\n').count())
        });
        for field in SV_FIELD_RE.captures_iter(&body_wo_fn) {
            let Some(type_tok) = field.get(1) else {
                continue;
            };
            // Count to the start of the type token, not the match start: `^\s*`
            // consumes leading newlines, which would resolve to the class line.
            let field_line = line + line_for(&body_wo_fn, type_tok.start()) - 1;
            for (ref_name, is_generic) in
                sv_collect_type_refs(type_tok.as_str(), false, &type_params)
            {
                let context = if is_generic { "generic_arg" } else { "field" };
                aug.add_ref_edge(
                    &class_nid,
                    &ref_name,
                    "references",
                    field_line,
                    Some(context),
                );
            }
        }

        for fm in SV_FUNC_RE.captures_iter(body) {
            let (Some(ret), Some(name), Some(params), Some(whole_fn)) =
                (fm.get(1), fm.get(2), fm.get(3), fm.get(0))
            else {
                continue;
            };
            let func_line = line + line_for(body, whole_fn.start()) - 1;
            let func_nid = make_id(&[class_nid.as_str(), name.as_str()]);
            aug.add_node(&func_nid, name.as_str(), func_line);
            aug.add_edge(&class_nid, &func_nid, "method", func_line);

            for (ref_name, is_generic) in sv_collect_type_refs(ret.as_str(), false, &type_params) {
                let context = if is_generic {
                    "generic_arg"
                } else {
                    "return_type"
                };
                aug.add_ref_edge(&func_nid, &ref_name, "references", func_line, Some(context));
            }
            for param in sv_split_type_list(params.as_str()) {
                let Some(pm) = SV_PARAM_RE.captures(&param) else {
                    continue;
                };
                for (ref_name, is_generic) in sv_collect_type_refs(&pm[1], false, &type_params) {
                    let context = if is_generic {
                        "generic_arg"
                    } else {
                        "parameter_type"
                    };
                    aug.add_ref_edge(&func_nid, &ref_name, "references", func_line, Some(context));
                }
            }
        }
    }
}
