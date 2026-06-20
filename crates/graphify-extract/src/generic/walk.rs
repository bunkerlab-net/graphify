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

use indexmap::IndexMap;
use serde_json::Value;
use tree_sitter::Node;

use crate::ids::{make_id, make_id1};
use crate::types::{Edge, Node as GNode};

use super::config::{LangConfig, LangId};
use super::inherit::{
    emit_cpp_inheritance, emit_csharp_inheritance, emit_java_inheritance, emit_kotlin_inheritance,
    emit_php_inheritance, emit_scala_inheritance, emit_swift_inheritance, emit_ts_inheritance,
};
use super::js_extra::{
    JsAssignTarget, is_js_function_value, js_extra_walk, js_member_assignment_target,
};
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

// ── Small AST helpers ──────────────────────────────────────────────────────────

/// Collect the named children of `node` into a `Vec`.
#[must_use]
pub(crate) fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().is_named() {
                out.push(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

/// Return the first child of `node` whose kind is `kind`.
#[must_use]
pub(crate) fn first_child_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().kind() == kind {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// `true` if any child of `node` has the given `kind` (allocation-free).
#[must_use]
pub(super) fn any_child_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().kind() == kind {
                return true;
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    false
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
        PHP_TYPE_NODE_KINDS, RefRole, c_collect_type_refs, cpp_collect_type_refs,
        csharp_attribute_names, csharp_collect_type_refs, java_collect_type_refs,
        java_method_annotation_names, kotlin_collect_type_refs, kotlin_function_return_type_node,
        php_collect_type_refs, php_method_return_type_node, python_collect_param_refs,
        python_collect_type_refs, scala_collect_type_refs, swift_collect_type_refs,
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
            | LangId::Swift
            | LangId::Php
            | LangId::Kotlin
            | LangId::Scala
            | LangId::C
            | LangId::Cpp
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
        LangId::Swift => {
            // Swift parameters are direct `parameter` children of the function.
            for p in named_children(node) {
                if p.kind() == "parameter"
                    && let Some(type_node) = p.child_by_field_name("type")
                {
                    let mut refs: Vec<(String, RefRole)> = Vec::new();
                    swift_collect_type_refs(type_node, source, false, &mut refs);
                    for (name, role) in refs {
                        emit_ref(ctx, &name, role.into_context("parameter_type"));
                    }
                }
            }
            if let Some(return_node) = node.child_by_field_name("return_type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                swift_collect_type_refs(return_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
        }
        LangId::Php => {
            if let Some(params) = first_child_kind(node, "formal_parameters") {
                for p in named_children(params) {
                    if p.kind() != "simple_parameter" {
                        continue;
                    }
                    if let Some(type_node) = named_children(p)
                        .into_iter()
                        .find(|c| PHP_TYPE_NODE_KINDS.contains(&c.kind()))
                    {
                        let mut refs: Vec<(String, RefRole)> = Vec::new();
                        php_collect_type_refs(type_node, source, false, &mut refs);
                        for (name, role) in refs {
                            emit_ref(ctx, &name, role.into_context("parameter_type"));
                        }
                    }
                }
            }
            if let Some(return_node) = php_method_return_type_node(node) {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                php_collect_type_refs(return_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
        }
        LangId::Kotlin => {
            if let Some(params) = first_child_kind(node, "function_value_parameters") {
                for p in named_children(params) {
                    if p.kind() != "parameter" {
                        continue;
                    }
                    if let Some(type_node) = named_children(p).into_iter().find(|c| {
                        matches!(c.kind(), "user_type" | "nullable_type" | "type_reference")
                    }) {
                        let mut refs: Vec<(String, RefRole)> = Vec::new();
                        kotlin_collect_type_refs(type_node, source, false, &mut refs);
                        for (name, role) in refs {
                            emit_ref(ctx, &name, role.into_context("parameter_type"));
                        }
                    }
                }
            }
            if let Some(return_node) = kotlin_function_return_type_node(node) {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                kotlin_collect_type_refs(return_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
        }
        LangId::Scala => {
            if let Some(params) = first_child_kind(node, "parameters") {
                for p in named_children(params) {
                    if p.kind() != "parameter" {
                        continue;
                    }
                    if let Some(type_node) = p.child_by_field_name("type") {
                        let mut refs: Vec<(String, RefRole)> = Vec::new();
                        scala_collect_type_refs(type_node, source, false, &mut refs);
                        for (name, role) in refs {
                            emit_ref(ctx, &name, role.into_context("parameter_type"));
                        }
                    }
                }
            }
            if let Some(return_node) = node.child_by_field_name("return_type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                scala_collect_type_refs(return_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
        }
        LangId::C | LangId::Cpp => {
            let collect: super::references::RefCollector = if lang == LangId::Cpp {
                cpp_collect_type_refs
            } else {
                c_collect_type_refs
            };
            if let Some(return_node) = node.child_by_field_name("type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                collect(return_node, source, false, &mut refs);
                for (name, role) in refs {
                    emit_ref(ctx, &name, role.into_context("return_type"));
                }
            }
            // The function_declarator may be wrapped in pointer/reference declarators.
            let mut decl = node.child_by_field_name("declarator");
            while let Some(d) = decl {
                if matches!(d.kind(), "pointer_declarator" | "reference_declarator") {
                    decl = d.child_by_field_name("declarator");
                } else {
                    break;
                }
            }
            if let Some(d) = decl
                && d.kind() == "function_declarator"
                && let Some(params_node) = d.child_by_field_name("parameters")
            {
                for p in named_children(params_node) {
                    if p.kind() != "parameter_declaration" {
                        continue;
                    }
                    if let Some(ptype) = p.child_by_field_name("type") {
                        let mut refs: Vec<(String, RefRole)> = Vec::new();
                        collect(ptype, source, false, &mut refs);
                        for (name, role) in refs {
                            emit_ref(ctx, &name, role.into_context("parameter_type"));
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Emit `references` edges (context `field` or `generic_arg`) from a class
/// member's type node, using the supplied language type-ref `collect`or.
/// Self-references (member type == enclosing class) are skipped.
fn emit_member_type_refs(
    ctx: &mut WalkCtx<'_, '_>,
    type_node: Node<'_>,
    parent_nid: &str,
    line: u32,
    source: &[u8],
    collect: super::references::RefCollector,
) {
    let mut refs: Vec<(String, super::references::RefRole)> = Vec::new();
    collect(type_node, source, false, &mut refs);
    for (name, role) in refs {
        let target =
            ensure_named_node(&name, line, ctx.stem, ctx.str_path, ctx.nodes, ctx.seen_ids);
        if target != parent_nid {
            add_edge(
                parent_nid,
                &target,
                "references",
                line,
                ctx.str_path,
                Some(role.into_context("field")),
                ctx.edges,
            );
        }
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
    /// Names declared as `protocol` in the current Swift compilation unit, used
    /// to classify conformances as `implements`. Empty for non-Swift files.
    pub swift_protocol_names: &'a HashSet<String>,
    /// Names declared as `class`/`struct`/`enum`/`actor` in the current Swift
    /// compilation unit, used to classify a base as `inherits`. Empty for
    /// non-Swift files.
    pub swift_class_names: &'a HashSet<String>,
    /// PHP event-listener edges (`$listen`/`$subscribe` arrays) collected during
    /// the structural walk and resolved to `listened_by` edges after every node
    /// exists. `(event_class, listener_class, line)`. Empty for non-PHP files.
    pub pending_listen_edges: &'a mut Vec<(String, String, u32)>,
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
        // Swift `import CoreKit` names a module, not a file path, so there is no
        // existing node for the edge to point at. Materialize a `type=module`
        // anchor node — shared across every file importing the same module via
        // its stable id — so build_from_json doesn't prune the `imports` edge as
        // a dangling/external reference (#1327).
        if config.lang_id == LangId::Swift
            && let Some((mod_nid, mod_label)) =
                crate::import_handlers::swift_import_module(node, source)
            && ctx.seen_ids.insert(mod_nid.clone())
        {
            let line = node.start_position().row as u32 + 1;
            let mut metadata = IndexMap::new();
            metadata.insert("type".to_string(), Value::String("module".to_string()));
            ctx.nodes.push(GNode {
                id: mod_nid,
                label: mod_label,
                file_type: "code".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: Some(metadata),
            });
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

        // PHP extends/implements/use
        if config.lang_id == LangId::Php {
            emit_php_inheritance(ctx, node, source, &class_nid, line);
        }

        // Kotlin delegation_specifiers
        if config.lang_id == LangId::Kotlin {
            emit_kotlin_inheritance(ctx, node, source, &class_nid, line);
        }

        // Scala extends_clause + constructor parameters
        if config.lang_id == LangId::Scala {
            emit_scala_inheritance(ctx, node, source, &class_nid, line);
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

    // ── PHP property_declaration ──────────────────────────────────────────────
    if config.lang_id == LangId::Php
        && t == "property_declaration"
        && let Some(parent) = parent_class_nid
    {
        // Event-listener arrays ($listen/$subscribe = [Event::class => [Listener::class]])
        // defer `listened_by` edges until every node exists (resolved in the
        // orchestrator). Mirrors graphify-py's property_declaration listener pass.
        if !config.event_listener_properties.is_empty()
            && collect_php_event_listeners(node, source, config, ctx.pending_listen_edges)
        {
            return;
        }
        let line = node.start_position().row as u32 + 1;
        if let Some(type_node) = named_children(node)
            .into_iter()
            .find(|c| super::references::PHP_TYPE_NODE_KINDS.contains(&c.kind()))
        {
            emit_member_type_refs(
                ctx,
                type_node,
                parent,
                line,
                source,
                super::references::php_collect_type_refs,
            );
        }
        return;
    }

    // ── Kotlin property_declaration ───────────────────────────────────────────
    if config.lang_id == LangId::Kotlin
        && t == "property_declaration"
        && let Some(parent) = parent_class_nid
    {
        let line = node.start_position().row as u32 + 1;
        if let Some(type_node) = super::references::kotlin_property_type_node(node) {
            emit_member_type_refs(
                ctx,
                type_node,
                parent,
                line,
                source,
                super::references::kotlin_collect_type_refs,
            );
        }
        return;
    }

    // ── Swift property_declaration ────────────────────────────────────────────
    if config.lang_id == LangId::Swift
        && t == "property_declaration"
        && let Some(parent) = parent_class_nid
    {
        let line = node.start_position().row as u32 + 1;
        if let Some(type_anno) = super::references::swift_property_type_node(node) {
            emit_member_type_refs(
                ctx,
                type_anno,
                parent,
                line,
                source,
                super::references::swift_collect_type_refs,
            );
        }
        // #1356 Stage 1: a constructor call in a property initializer
        // (`let vm = VM()`) lives outside any function body, so the call-graph
        // pass never reaches it. Queue each initializer call node so it is walked
        // with the enclosing type as caller, producing a `calls` edge to the
        // constructed type via cross-file resolution.
        for child in named_children(node) {
            if config.call_types.contains(&child.kind()) {
                ctx.function_bodies.push((parent.to_string(), child));
            }
        }
        return;
    }

    // ── Scala val_definition ──────────────────────────────────────────────────
    // Falls through (no early return) so call expressions in the initializer
    // are still walked.
    if config.lang_id == LangId::Scala
        && t == "val_definition"
        && let Some(parent) = parent_class_nid
        && let Some(type_node) = node.child_by_field_name("type")
    {
        let line = node.start_position().row as u32 + 1;
        emit_member_type_refs(
            ctx,
            type_node,
            parent,
            line,
            source,
            super::references::scala_collect_type_refs,
        );
    }

    // ── C++ field_declaration ─────────────────────────────────────────────────
    if config.lang_id == LangId::Cpp
        && t == "field_declaration"
        && let Some(parent) = parent_class_nid
    {
        // Skip method prototypes (a field_declaration with a function_declarator
        // is a member-function declaration, not a data member) when emitting
        // type references.
        let mut dcur = node.walk();
        let is_method = node
            .children_by_field_name("declarator", &mut dcur)
            .any(|d| {
                d.kind() == "function_declarator"
                    || (matches!(d.kind(), "pointer_declarator" | "reference_declarator")
                        && any_child_kind(d, "function_declarator"))
            });
        if !is_method && let Some(type_node) = node.child_by_field_name("type") {
            let line = node.start_position().row as u32 + 1;
            emit_member_type_refs(
                ctx,
                type_node,
                parent,
                line,
                source,
                super::references::cpp_collect_type_refs,
            );
        }
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
            // JS/TS: capture `this.X = () => {}` / `this.X = function(){}`
            // assigned directly in this function/constructor body. They live
            // inside the body (otherwise only walked for calls), so without this
            // they are never emitted. Owner is the enclosing class when present
            // (a constructor's methods belong to the class), else the function
            // itself. Mirrors graphify-py `_extract_generic` (#09da529).
            if matches!(
                config.lang_id,
                LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
            ) {
                let this_owner: &str = parent_class_nid.unwrap_or(&func_nid);
                let mut bcur = body.walk();
                if bcur.goto_first_child() {
                    loop {
                        let stmt = bcur.node();
                        if stmt.kind() == "expression_statement"
                            && let Some(assign) = first_child_kind(stmt, "assignment_expression")
                            && let Some(val) = assign.child_by_field_name("right")
                            && is_js_function_value(val.kind())
                            && let Some(left) = assign.child_by_field_name("left")
                            && let Some(JsAssignTarget::This(m_name)) =
                                js_member_assignment_target(left, source)
                        {
                            let m_line = stmt.start_position().row as u32 + 1;
                            let m_nid = make_id(&[this_owner, m_name.as_str()]);
                            add_node(
                                &m_nid,
                                &format!(".{m_name}()"),
                                m_line,
                                str_path,
                                ctx.nodes,
                                ctx.seen_ids,
                            );
                            add_edge(
                                this_owner, &m_nid, "method", m_line, str_path, None, ctx.edges,
                            );
                            if let Some(m_body) = val.child_by_field_name("body") {
                                ctx.function_bodies.push((m_nid, m_body));
                            }
                        }
                        if !bcur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
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

/// Return the class (scope) name of a PHP `Foo::BAR` / `Foo::class` / `Foo::$bar`
/// access node: the `scope` field, else the first named `name`/`qualified_name`/
/// `identifier` child. Mirrors graphify-py `_php_class_const_scope`.
pub(super) fn php_class_const_scope(node: Node<'_>, source: &[u8]) -> Option<String> {
    let scope = node.child_by_field_name("scope").or_else(|| {
        named_children(node)
            .into_iter()
            .find(|c| matches!(c.kind(), "name" | "qualified_name" | "identifier"))
    });
    scope.map(|s| read_text_owned(s, source))
}

/// Collect PHP event-listener edges from a `$listen`/`$subscribe` property array
/// (`[Event::class => [Listener::class, ...]]`) into `pending_listen_edges`.
///
/// Returns `true` when a listener property was handled (so the caller stops
/// descending). Mirrors the `property_declaration` listener branch in
/// graphify-py `_extract_generic`'s structural `walk`.
fn collect_php_event_listeners(
    node: Node<'_>,
    source: &[u8],
    config: &LangConfig,
    pending_listen_edges: &mut Vec<(String, String, u32)>,
) -> bool {
    let mut handled = false;
    for element in named_children(node) {
        if element.kind() != "property_element" {
            continue;
        }
        let mut prop_name: Option<String> = None;
        let mut array_node: Option<Node<'_>> = None;
        for c in named_children(element) {
            match c.kind() {
                "variable_name" => {
                    if let Some(name) = named_children(c).into_iter().find(|n| n.kind() == "name") {
                        prop_name = Some(read_text_owned(name, source));
                    }
                }
                "array_creation_expression" => array_node = Some(c),
                _ => {}
            }
        }
        let (Some(prop_name), Some(array_node)) = (prop_name, array_node) else {
            continue;
        };
        if !config
            .event_listener_properties
            .contains(&prop_name.as_str())
        {
            continue;
        }
        handled = true;
        for entry in named_children(array_node) {
            if entry.kind() != "array_element_initializer" {
                continue;
            }
            let mut event_cls: Option<String> = None;
            let mut listener_arr: Option<Node<'_>> = None;
            for sub in named_children(entry) {
                if sub.kind() == "class_constant_access_expression" && event_cls.is_none() {
                    event_cls = php_class_const_scope(sub, source);
                } else if sub.kind() == "array_creation_expression" {
                    listener_arr = Some(sub);
                }
            }
            let (Some(event_cls), Some(listener_arr)) = (event_cls, listener_arr) else {
                continue;
            };
            for listener_entry in named_children(listener_arr) {
                if listener_entry.kind() != "array_element_initializer" {
                    continue;
                }
                if let Some(item) = named_children(listener_entry)
                    .into_iter()
                    .find(|i| i.kind() == "class_constant_access_expression")
                    && let Some(listener_cls) = php_class_const_scope(item, source)
                {
                    let line = item.start_position().row as u32 + 1;
                    pending_listen_edges.push((event_cls.clone(), listener_cls, line));
                }
            }
        }
    }
    handled
}
