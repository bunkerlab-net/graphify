//! Main tree-sitter structural walk.
//!
//! `walk` is the recursive descent that builds nodes/edges for classes,
//! functions, imports, and language-specific constructs.
//! Low-level graph helpers (`add_node`, `add_edge`) live here because every
//! other submodule needs them.

// Tree-sitter row numbers represent source line indices; files with 2^32+
// lines do not exist in practice, so usize→u32 truncation is safe.
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashSet;

use tree_sitter::Node;

use crate::ids::{make_id, make_id1};
use crate::types::{Edge, Node as GNode};

use super::config::{LangConfig, LangId};
use super::inherit::{
    emit_cpp_inheritance, emit_csharp_inheritance, emit_java_inheritance, emit_swift_inheritance,
    emit_ts_inheritance,
};
use super::js_extra::js_extra_walk;
use super::names::{get_cpp_func_name, read_csharp_type_name, read_text_owned};

// ── Graph helpers ─────────────────────────────────────────────────────────────

/// Insert a new graph node if `nid` has not been seen before.
///
/// The `seen_ids` set is the deduplication gate — a second call with the same
/// `nid` is silently dropped so that multiple structural passes (e.g.
/// file-level node + function-level) cannot produce duplicate node entries.
pub(super) fn add_node(
    nid: &str,
    label: &str,
    line: u32,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) {
    if seen_ids.insert(nid.to_string()) {
        nodes.push(GNode {
            id: nid.to_string(),
            label: label.to_string(),
            file_type: "code".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            metadata: None,
        });
    }
}

/// Append an edge to the edge list.
///
/// Unlike nodes, edges are not deduplicated here — the caller is responsible
/// for deduplication via `seen_call_pairs` or the final clean pass in
/// [`extract_generic`].
pub(super) fn add_edge(
    src: &str,
    tgt: &str,
    relation: &str,
    line: u32,
    str_path: &str,
    context: Option<&str>,
    edges: &mut Vec<Edge>,
) {
    edges.push(Edge {
        external: false,
        source: src.to_string(),
        target: tgt.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: context.map(str::to_string),
        confidence_score: None,
    });
}

// ── Body finder ───────────────────────────────────────────────────────────────

/// Locate the body child of a class or function node.
///
/// First tries the grammar's `body` field; falls back to scanning for a child
/// whose kind appears in `config.body_fallback_child_types`. The fallback is
/// needed for languages like Kotlin whose grammar uses `class_body` or
/// `function_body` node types rather than a named field.
pub(super) fn find_body<'tree>(node: Node<'tree>, config: &LangConfig) -> Option<Node<'tree>> {
    if let Some(b) = node.child_by_field_name(config.body_field) {
        return Some(b);
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if config.body_fallback_child_types.contains(&child.kind()) {
                return Some(child);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

// ── ensure_named_node ─────────────────────────────────────────────────────────

/// Return the NID for a named entity, creating a placeholder node if needed.
///
/// First checks for a file-qualified ID (`<stem>_<name>`); if already seen,
/// returns that ID. Otherwise ensures the bare-name node exists (creating it
/// when absent) and returns the bare NID. Used for cross-file type references
/// in C# `field_declaration` processing.
pub(super) fn ensure_named_node(
    name: &str,
    line: u32,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) -> String {
    let nid1 = make_id(&[stem, name]);
    if seen_ids.contains(&nid1) {
        return nid1;
    }
    let nid2 = make_id1(name);
    if !seen_ids.contains(&nid2) {
        add_node(&nid2, name, line, str_path, nodes, seen_ids);
    }
    nid2
}

// ── Function-level reference edges ────────────────────────────────────────────

/// Emit `references` edges with a `context` attribute from a function or
/// method declaration's parameter list, return type, and decorations
/// (annotations / attributes). Active for Python, C#, Java, JavaScript, and
/// TypeScript (`.ts` and `.tsx`). Plain JS function declarations have no
/// type annotations, so the TS/JS branch is effectively a no-op there.
///
/// Mirrors the per-language reference passes added to `_extract_generic` in
/// `graphify-py` @ ab4e542.
#[allow(clippy::too_many_lines)] // linear per-language dispatch — splitting would hide the parallel shape between Python/C#/Java/TS
fn emit_function_reference_edges(
    ctx: &mut WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    func_nid: &str,
    line: u32,
) {
    use super::references::{
        RefRole, csharp_attribute_names, csharp_collect_type_refs, java_collect_type_refs,
        java_method_annotation_names, python_collect_param_refs, python_collect_type_refs,
        ts_collect_type_refs,
    };

    let lang = ctx.config.lang_id;
    if !matches!(
        lang,
        LangId::Python
            | LangId::CSharp
            | LangId::Java
            | LangId::TypeScript
            | LangId::TypeScriptX
            | LangId::JavaScript
    ) {
        return;
    }
    let stem = ctx.stem;
    let str_path = ctx.str_path;

    // Helper: lazily ensure the target node exists and emit the edge.
    // Skips self-references (e.g. a recursive call where the parameter type
    // is the enclosing class itself — the structural pass already handles
    // those via `method` edges).
    let emit_ref = |ctx: &mut WalkCtx<'_, '_>, ref_name: &str, ctx_kind: &str| {
        let target =
            super::inherit::emit_base_node(ref_name, line, stem, str_path, ctx.nodes, ctx.seen_ids);
        if target == func_nid {
            return;
        }
        add_edge(
            func_nid,
            &target,
            "references",
            line,
            str_path,
            Some(ctx_kind),
            ctx.edges,
        );
    };

    match lang {
        LangId::Python => {
            let params_node = node.child_by_field_name("parameters");
            for (name, role) in python_collect_param_refs(params_node, source) {
                emit_ref(ctx, &name, role.into_context("parameter_type"));
            }
            if let Some(return_type_node) = node.child_by_field_name("return_type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                python_collect_type_refs(return_type_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
        }
        LangId::CSharp => {
            if let Some(params_node) = node.child_by_field_name("parameters") {
                let mut cur = params_node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "parameter"
                            && let Some(type_node) = cur.node().child_by_field_name("type")
                        {
                            let mut refs: Vec<(String, RefRole)> = Vec::new();
                            csharp_collect_type_refs(type_node, source, false, &mut refs);
                            for (name, role) in refs {
                                emit_ref(ctx, &name, role.into_context("parameter_type"));
                            }
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if let Some(return_node) = node.child_by_field_name("returns") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                csharp_collect_type_refs(return_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
            for attr_name in csharp_attribute_names(node, source) {
                emit_ref(ctx, &attr_name, "attribute");
            }
        }
        LangId::Java => {
            if let Some(params_node) = node.child_by_field_name("parameters") {
                let mut cur = params_node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "formal_parameter"
                            && let Some(type_node) = cur.node().child_by_field_name("type")
                        {
                            let mut refs: Vec<(String, RefRole)> = Vec::new();
                            java_collect_type_refs(type_node, source, false, &mut refs);
                            for (name, role) in refs {
                                emit_ref(ctx, &name, role.into_context("parameter_type"));
                            }
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if let Some(return_node) = node.child_by_field_name("type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                java_collect_type_refs(return_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
            for anno_name in java_method_annotation_names(node, source) {
                emit_ref(ctx, &anno_name, "attribute");
            }
        }
        LangId::TypeScript | LangId::TypeScriptX | LangId::JavaScript => {
            // TS/TSX method signatures expose params via a `parameters` field
            // whose children are `required_parameter` / `optional_parameter`,
            // each carrying a `type` field of kind `type_annotation`. The
            // return type sits on the function node itself via the
            // `return_type` field. Plain JS function declarations have no
            // type annotations and are silently no-ops.
            if let Some(params_node) = node.child_by_field_name("parameters") {
                let mut cur = params_node.walk();
                if cur.goto_first_child() {
                    loop {
                        if matches!(
                            cur.node().kind(),
                            "required_parameter" | "optional_parameter"
                        ) && let Some(type_node) = cur.node().child_by_field_name("type")
                        {
                            let mut refs: Vec<(String, RefRole)> = Vec::new();
                            ts_collect_type_refs(type_node, source, false, &mut refs);
                            for (name, role) in refs {
                                emit_ref(ctx, &name, role.into_context("parameter_type"));
                            }
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if let Some(return_type_node) = node.child_by_field_name("return_type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                ts_collect_type_refs(return_type_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
        }
        _ => {}
    }
}

// ── Structural walk ───────────────────────────────────────────────────────────

/// Shared state threaded through every structural-walk recursion.
pub(super) struct WalkCtx<'a, 'tree> {
    pub config: &'a LangConfig,
    pub file_nid: &'a str,
    pub stem: &'a str,
    pub str_path: &'a str,
    pub nodes: &'a mut Vec<GNode>,
    pub edges: &'a mut Vec<Edge>,
    pub seen_ids: &'a mut HashSet<String>,
    pub function_bodies: &'a mut Vec<(String, Node<'tree>)>,
    /// Set of identifiers declared as `interface` in the current C# compilation
    /// unit. Pre-computed before the structural walk so the inheritance emitter
    /// can split `inherits` (class extension) from `implements` (interface
    /// implementation) on `base_list` entries. Empty for non-C# files.
    pub csharp_interface_names: &'a HashSet<String>,
}

/// Recursive structural AST walk that emits nodes and edges for classes,
/// functions, imports, and language-specific constructs.
///
/// `clippy::too_many_lines` is suppressed: this is a linear dispatch over many
/// AST node kinds; splitting it fragments the per-kind branches without
/// isolating a reusable shape.
#[allow(clippy::too_many_lines)]
pub(super) fn walk<'tree>(
    ctx: &mut WalkCtx<'_, 'tree>,
    node: Node<'tree>,
    parent_class_nid: Option<&str>,
    source: &[u8],
) {
    // Re-bind read-only fields as locals so the function body reads naturally.
    // Mutable fields stay accessed via `ctx.<field>` so recursive `walk(ctx, …)`
    // calls remain valid.
    let config: &LangConfig = ctx.config;
    let file_nid: &str = ctx.file_nid;
    let stem: &str = ctx.stem;
    let str_path: &str = ctx.str_path;
    let t = node.kind();

    // ── Imports ──────────────────────────────────────────────────────────────
    if config.import_types.contains(&t) {
        if let Some(handler) = config.import_handler {
            handler(source, node, file_nid, stem, str_path, ctx.edges);
        }
        // `export_statement` may also wrap a declaration body
        // (`export function App() {}` / `export class Foo {}`) — keep
        // walking its children so the wrapped declaration is extracted.
        // Pure `import_statement` and `import_from_statement` never carry
        // child declarations and can return immediately.
        if t != "export_statement" {
            return;
        }
    }

    // ── Classes ──────────────────────────────────────────────────────────────
    if config.class_types.contains(&t) {
        // Resolve class name
        let name_node = node.child_by_field_name(config.name_field).or_else(|| {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if config.name_fallback_child_types.contains(&child.kind()) {
                        return Some(child);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        });
        let Some(name_node) = name_node else { return };
        let class_name = read_text_owned(name_node, source);
        let class_nid = make_id(&[stem, &class_name]);
        let line = node.start_position().row as u32 + 1;
        add_node(
            &class_nid,
            &class_name,
            line,
            str_path,
            ctx.nodes,
            ctx.seen_ids,
        );
        add_edge(
            file_nid, &class_nid, "contains", line, str_path, None, ctx.edges,
        );

        // Python inheritance
        if config.lang_id == LangId::Python
            && let Some(args) = node.child_by_field_name("superclasses")
        {
            let mut cur = args.walk();
            if cur.goto_first_child() {
                loop {
                    let arg = cur.node();
                    if arg.kind() == "identifier" {
                        let base = read_text_owned(arg, source);
                        let base_nid = if ctx.seen_ids.contains(&make_id(&[stem, &base])) {
                            make_id(&[stem, &base])
                        } else {
                            let bn = make_id1(&base);
                            if !ctx.seen_ids.contains(&bn) {
                                ctx.nodes.push(GNode {
                                    id: bn.clone(),
                                    label: base.clone(),
                                    file_type: "code".to_string(),
                                    source_file: String::new(),
                                    source_location: None,
                                    metadata: None,
                                });
                                ctx.seen_ids.insert(bn.clone());
                            }
                            bn
                        };
                        add_edge(
                            &class_nid, &base_nid, "inherits", line, str_path, None, ctx.edges,
                        );
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        // Swift conformance/inheritance
        if config.lang_id == LangId::Swift {
            emit_swift_inheritance(ctx, node, source, &class_nid, line);
        }

        // C# base_list
        if config.lang_id == LangId::CSharp {
            emit_csharp_inheritance(ctx, node, source, &class_nid, line);
        }

        // Java extends/implements
        if config.lang_id == LangId::Java {
            emit_java_inheritance(ctx, node, source, &class_nid, t, line);
        }

        // C++ base_class_clause
        if config.lang_id == LangId::Cpp {
            emit_cpp_inheritance(ctx, node, source, &class_nid, line);
        }

        // TS/JS class_heritage (extends_clause + implements_clause)
        if matches!(
            config.lang_id,
            LangId::TypeScript | LangId::TypeScriptX | LangId::JavaScript
        ) {
            emit_ts_inheritance(ctx, node, source, &class_nid, line);
        }

        // Find body and recurse
        if let Some(body) = find_body(node, config) {
            let mut cur = body.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    walk(ctx, child, Some(&class_nid), source);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        return;
    }

    // ── C# field_declaration ─────────────────────────────────────────────────
    if config.lang_id == LangId::CSharp
        && t == "field_declaration"
        && let Some(parent) = parent_class_nid
    {
        let type_node = node.child_by_field_name("type").or_else(|| {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "variable_declaration" {
                        return child.child_by_field_name("type");
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        });
        if let Some(type_name) = read_csharp_type_name(type_node, source)
            && !type_name.is_empty()
        {
            let line = node.start_position().row as u32 + 1;
            let tgt = ensure_named_node(&type_name, line, stem, str_path, ctx.nodes, ctx.seen_ids);
            let e = Edge {
                external: false,
                source: parent.to_string(),
                target: tgt,
                relation: "references".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: Some("field".to_string()),
                confidence_score: None,
            };
            ctx.edges.push(e);
        }
        return;
    }

    // ── C++ field_declaration ─────────────────────────────────────────────────
    if config.lang_id == LangId::Cpp
        && t == "field_declaration"
        && let Some(parent) = parent_class_nid
    {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if (child.kind() == "field_identifier"
                    || matches!(
                        child.kind(),
                        "pointer_declarator"
                            | "reference_declarator"
                            | "init_declarator"
                            | "array_declarator"
                            | "identifier"
                    ))
                    && let Some(name) = get_cpp_func_name(child, source)
                {
                    let line = child.start_position().row as u32 + 1;
                    let field_nid = make_id(&[parent, &name]);
                    add_node(&field_nid, &name, line, str_path, ctx.nodes, ctx.seen_ids);
                    let e = Edge {
                        external: false,
                        source: parent.to_string(),
                        target: field_nid,
                        relation: "defines".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: Some("field".to_string()),
                        confidence_score: None,
                    };
                    ctx.edges.push(e);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // ── Functions ─────────────────────────────────────────────────────────────
    if config.function_types.contains(&t) {
        let func_name: Option<String> = match t {
            "deinit_declaration" => Some("deinit".to_string()),
            "subscript_declaration" => Some("subscript".to_string()),
            _ if let Some(resolver) = config.resolve_function_name => node
                .child_by_field_name("declarator")
                .and_then(|d| resolver(d, source)),
            _ => {
                let nn = node.child_by_field_name(config.name_field).or_else(|| {
                    let mut cur = node.walk();
                    if cur.goto_first_child() {
                        loop {
                            let child = cur.node();
                            if config.name_fallback_child_types.contains(&child.kind()) {
                                return Some(child);
                            }
                            if !cur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                    None
                });
                nn.map(|n| read_text_owned(n, source))
            }
        };

        let Some(func_name) = func_name else { return };
        if func_name.is_empty() {
            return;
        }

        let line = node.start_position().row as u32 + 1;
        let (func_nid, label, parent_nid) = if let Some(parent) = parent_class_nid {
            (
                make_id(&[parent, &func_name]),
                format!(".{func_name}()"),
                parent.to_string(),
            )
        } else {
            (
                make_id(&[stem, &func_name]),
                format!("{func_name}()"),
                file_nid.to_string(),
            )
        };

        add_node(&func_nid, &label, line, str_path, ctx.nodes, ctx.seen_ids);
        let relation = if parent_class_nid.is_some() {
            "method"
        } else {
            "contains"
        };
        add_edge(
            &parent_nid,
            &func_nid,
            relation,
            line,
            str_path,
            None,
            ctx.edges,
        );

        emit_function_reference_edges(ctx, node, source, &func_nid, line);

        if let Some(body) = find_body(node, config) {
            ctx.function_bodies.push((func_nid, body));
        }
        return;
    }

    // ── JS/TS extra walk (arrow functions, CJS require) ───────────────────────
    if (config.lang_id == LangId::JavaScript
        || config.lang_id == LangId::TypeScript
        || config.lang_id == LangId::TypeScriptX)
        && js_extra_walk(ctx, node, source, parent_class_nid)
    {
        return;
    }

    // ── C# namespace_declaration ──────────────────────────────────────────────
    if config.lang_id == LangId::CSharp && t == "namespace_declaration" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let ns_name = read_text_owned(name_node, source);
            let ns_nid = make_id(&[stem, &ns_name]);
            let line = node.start_position().row as u32 + 1;
            add_node(&ns_nid, &ns_name, line, str_path, ctx.nodes, ctx.seen_ids);
            add_edge(
                file_nid, &ns_nid, "contains", line, str_path, None, ctx.edges,
            );
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut cur = body.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    walk(ctx, child, parent_class_nid, source);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        return;
    }

    // ── Swift enum_entry ──────────────────────────────────────────────────────
    if config.lang_id == LangId::Swift
        && t == "enum_entry"
        && let Some(parent) = parent_class_nid
    {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "simple_identifier" {
                    let case_name = read_text_owned(child, source);
                    let case_nid = make_id(&[parent, &case_name]);
                    let line = node.start_position().row as u32 + 1;
                    add_node(
                        &case_nid,
                        &case_name,
                        line,
                        str_path,
                        ctx.nodes,
                        ctx.seen_ids,
                    );
                    add_edge(
                        parent, &case_nid, "case_of", line, str_path, None, ctx.edges,
                    );
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // ── Python decorated_definition: transparent wrapper ──────────────────────
    // Python's `@property` / `@staticmethod` / `@classmethod` wrap the inner
    // `function_definition` in a `decorated_definition` node. The default recurse
    // below clears `parent_class_nid`, which would emit the inner method with a
    // class-unqualified id (e.g. `file_baz` instead of `file_bar_baz`). That
    // diverges from the class-qualified id the rationale walker uses for the same
    // method's docstring, leaving the rationale edge dangling and the docstring
    // node orphaned (#1050). Treat `decorated_definition` as transparent so
    // `parent_class_nid` propagates to the real function node.
    if t == "decorated_definition" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                walk(ctx, child, parent_class_nid, source);
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // ── Default: recurse ──────────────────────────────────────────────────────
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            walk(ctx, child, None, source);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
