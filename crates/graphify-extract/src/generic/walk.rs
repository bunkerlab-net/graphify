//! Main tree-sitter structural walk.
//!
//! `walk` is the recursive descent that builds nodes/edges for classes,
//! functions, imports, and language-specific constructs.
//! Low-level graph helpers (`add_node`, `add_edge`) live here because every
//! other submodule needs them.

// Tree-sitter row numbers represent source line indices; files with 2^32+
// lines do not exist in practice, so usize→u32 truncation is safe.
#![allow(clippy::cast_possible_truncation)]

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use serde_json::Value;
use tree_sitter::Node;

use crate::ids::{make_id, make_id1};
use crate::types::{Edge, Node as GNode};

use super::config::{LangConfig, LangId};
use super::indirect::{js_local_bound_names, python_local_bound_names};
use super::inherit::{
    emit_cpp_inheritance, emit_csharp_inheritance, emit_java_inheritance, emit_kotlin_inheritance,
    emit_php_inheritance, emit_ruby_inheritance, emit_scala_inheritance, emit_swift_inheritance,
    emit_ts_inheritance,
};
use super::js_extra::{
    JsAssignTarget, emit_ts_decorator_edges, is_js_function_value, js_extra_walk,
    js_member_assignment_target,
};
use super::names::{get_cpp_func_name, read_csharp_type_name, read_text_owned};

pub(crate) use super::graph::{
    add_edge, add_edge_meta, add_node, any_child_kind, ensure_named_node, find_body,
    first_child_kind, named_children,
};

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
    parent_class_nid: Option<&str>,
) {
    use super::references::{
        CsharpTypeRef, PHP_TYPE_NODE_KINDS, RefRole, c_collect_type_refs, cpp_collect_type_refs,
        csharp_attribute_names, csharp_collect_type_refs, java_annotation_names,
        java_collect_type_refs, kotlin_collect_type_refs, kotlin_function_return_type_node,
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
            // Materialise a (possibly sourceless) target stub for a C# type ref
            // and emit a `references` edge carrying ref_token/qualified metadata.
            let emit_cs = |ctx: &mut WalkCtx<'_, '_>, r: &CsharpTypeRef, context: &'static str| {
                let target = super::inherit::emit_base_node(
                    &r.name,
                    line,
                    stem,
                    str_path,
                    ctx.nodes,
                    ctx.seen_ids,
                );
                if target != func_nid {
                    push_csharp_ref_edge(ctx, func_nid, target, r, context, line);
                }
            };
            if let Some(params_node) = node.child_by_field_name("parameters") {
                let mut cur = params_node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "parameter"
                            && let Some(type_node) = cur.node().child_by_field_name("type")
                        {
                            let mut refs: Vec<CsharpTypeRef> = Vec::new();
                            csharp_collect_type_refs(type_node, source, false, &mut refs);
                            for r in &refs {
                                emit_cs(ctx, r, r.role.into_context("parameter_type"));
                            }
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if let Some(return_node) = node.child_by_field_name("returns") {
                let mut refs: Vec<CsharpTypeRef> = Vec::new();
                csharp_collect_type_refs(return_node, source, false, &mut refs);
                for r in &refs {
                    emit_cs(ctx, r, r.role.into_context("return_type"));
                }
            }
            for r in csharp_attribute_names(node, source) {
                emit_cs(ctx, &r, "attribute");
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
            for anno_name in java_annotation_names(node, source) {
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
                                // A builtin global type (Date, Promise, Map, …)
                                // must not casefold-bind a same-named user class
                                // (#1726); skip it as the resolvers do.
                                if crate::builtins::is_language_builtin_global(&name) {
                                    continue;
                                }
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
                    if crate::builtins::is_language_builtin_global(&name) {
                        continue;
                    }
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
                    // PHP 8 constructor property promotion parses a promoted param
                    // as `property_promotion_parameter` (type in the same shape); a
                    // promoted param is additionally a real class field (51f805e).
                    let is_promoted = p.kind() == "property_promotion_parameter";
                    if !is_promoted && p.kind() != "simple_parameter" {
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
                            // A promoted param also declares a class field; mirror
                            // the property_declaration field edge so the type is
                            // discoverable as a class field too (51f805e).
                            if is_promoted && let Some(parent) = parent_class_nid {
                                let target = super::inherit::emit_base_node(
                                    &name,
                                    line,
                                    stem,
                                    str_path,
                                    ctx.nodes,
                                    ctx.seen_ids,
                                );
                                if target != parent {
                                    add_edge(
                                        parent,
                                        &target,
                                        "references",
                                        line,
                                        str_path,
                                        Some(role.into_context("field")),
                                        ctx.edges,
                                    );
                                }
                            }
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

/// Push a C# `references` edge from `src` to `target` carrying `ref_token` /
/// `qualified` / `ref_qualifier` metadata (#1562). `qualified`/`ref_qualifier`
/// are omitted when false/empty. `context` is the ref-role context.
fn push_csharp_ref_edge(
    ctx: &mut WalkCtx<'_, '_>,
    src: &str,
    target: String,
    r: &super::references::CsharpTypeRef,
    context: &'static str,
    line: u32,
) {
    let mut pairs: Vec<(&str, Value)> = vec![("ref_token", Value::String(r.name.clone()))];
    if r.qualified {
        pairs.push(("qualified", Value::Bool(true)));
    }
    if !r.qualifier.is_empty() {
        pairs.push(("ref_qualifier", Value::String(r.qualifier.clone())));
    }
    let str_path = ctx.str_path;
    ctx.edges.push(Edge {
        external: false,
        source: src.to_string(),
        target,
        relation: "references".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: Some(context.to_string()),
        confidence_score: None,
        deferred: false,
        metadata: sanitized_metadata(pairs),
    });
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
        let target = ensure_named_node(&name, ctx.stem, ctx.str_path, ctx.nodes, ctx.seen_ids);
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

/// Emit C# member-type `references` edges (`field` / `generic_arg`) with
/// `ref_token`/`qualified`/`ref_qualifier` metadata (#1562), skipping a
/// self-reference to the enclosing class. The C# analogue of
/// [`emit_member_type_refs`], which cannot carry the qualifier metadata.
fn emit_csharp_member_type_refs(
    ctx: &mut WalkCtx<'_, '_>,
    type_node: Node<'_>,
    parent_nid: &str,
    line: u32,
    source: &[u8],
) {
    let mut refs: Vec<super::references::CsharpTypeRef> = Vec::new();
    super::references::csharp_collect_type_refs(type_node, source, false, &mut refs);
    for r in &refs {
        let target = ensure_named_node(&r.name, ctx.stem, ctx.str_path, ctx.nodes, ctx.seen_ids);
        if target != parent_nid {
            push_csharp_ref_edge(
                ctx,
                parent_nid,
                target,
                r,
                r.role.into_context("field"),
                line,
            );
        }
    }
}

/// Emit `references` edges for a Java record's header components
/// (`record Order(Payload p, List<Item> items, Attachment... rest)`), mirroring
/// field-type references. Type-parameter components are skipped by the
/// collector (#1519).
fn emit_java_record_component_refs(
    ctx: &mut WalkCtx<'_, '_>,
    record_node: Node<'_>,
    class_nid: &str,
    source: &[u8],
) {
    let Some(components) = record_node.child_by_field_name("parameters") else {
        return;
    };
    let mut cur = components.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let component = cur.node();
        let type_node = match component.kind() {
            "formal_parameter" => component.child_by_field_name("type"),
            "spread_parameter" => {
                // `Attachment... rest`: the type is the first named child that is
                // not the `modifiers` annotation block or the binder declarator.
                let mut found = None;
                let mut scur = component.walk();
                if scur.goto_first_child() {
                    loop {
                        let child = scur.node();
                        if child.is_named()
                            && !matches!(child.kind(), "modifiers" | "variable_declarator")
                        {
                            found = Some(child);
                            break;
                        }
                        if !scur.goto_next_sibling() {
                            break;
                        }
                    }
                }
                found
            }
            _ => None,
        };
        if let Some(type_node) = type_node {
            let component_line = component.start_position().row as u32 + 1;
            emit_member_type_refs(
                ctx,
                type_node,
                class_nid,
                component_line,
                source,
                super::references::java_collect_type_refs,
            );
        }
        if !cur.goto_next_sibling() {
            break;
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
    /// C# enclosing-namespace stack (dotted parts) — folded into C# type node ids
    /// and stamped as `namespace` metadata. Empty for every other language (#1562).
    pub csharp_ns_stack: &'a mut Vec<String>,
    /// C# lexical scope-id stack (one `s{start_byte}` per open namespace block),
    /// stamped as `scope_chain` metadata so a `using` binds only in its block.
    pub csharp_scope_stack: &'a mut Vec<String>,
    /// Ids of function / method / class definitions in this file — the callable
    /// defs an `indirect_call` reference may resolve to. Populated as each callable
    /// node is created; read by the same-file indirect capture and stamped as a
    /// durable `_callable` node marker for the cross-file resolver (#1565/#1566).
    pub callable_def_nids: &'a mut HashSet<String>,
    /// Python / JS-TS only: per-function set of names bound LOCALLY (params +
    /// assignment / for / with-as / comprehension targets). The indirect-dispatch
    /// shadow guard skips a call-argument identifier in the enclosing function's
    /// set, so a param / local shadowing a module fn name yields no edge. Empty for
    /// other languages.
    pub local_bound_names: &'a mut HashMap<String, HashSet<String>>,
}

/// C# namespace name from a `namespace_declaration` /
/// `file_scoped_namespace_declaration`: the `name` field, else the first
/// `identifier`/`qualified_name` child. Mirrors graphify-py `_csharp_namespace_name`.
fn csharp_namespace_name(node: Node<'_>, source: &[u8]) -> String {
    if let Some(name_node) = node.child_by_field_name("name") {
        return read_text_owned(name_node, source).trim().to_string();
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if matches!(c.kind(), "identifier" | "qualified_name") {
                return read_text_owned(c, source).trim().to_string();
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    String::new()
}

/// Canonical C# namespace node id: `csharp_namespace:` + first 16 hex of the
/// SHA-1 of the dotted name. Mirrors graphify-py `_csharp_namespace_id`.
#[must_use]
fn csharp_namespace_id(dotted: &str) -> String {
    use sha1::{Digest, Sha1};
    format!(
        "csharp_namespace:{}",
        &hex::encode(Sha1::digest(dotted.as_bytes()))[..16]
    )
}

/// Sanitised node/edge metadata map from key/value pairs (insertion order kept),
/// or `None` when empty. Routes values through the shared metadata sanitiser so
/// stamped source text can't inject markup (#1562).
pub(crate) fn sanitized_metadata(pairs: Vec<(&str, Value)>) -> Option<IndexMap<String, Value>> {
    if pairs.is_empty() {
        return None;
    }
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert(k.to_string(), v);
    }
    Some(
        graphify_security::sanitize_metadata_map(&map)
            .into_iter()
            .collect(),
    )
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
        // C#: `using` directives carry lexical-scope + kind metadata and need the
        // namespace scope stack, so they are emitted here rather than via the
        // generic import-handler slot (#1562).
        if config.lang_id == LangId::CSharp && t == "using_directive" {
            crate::import_handlers::import_csharp(
                source,
                node,
                file_nid,
                str_path,
                ctx.edges,
                ctx.csharp_scope_stack,
            );
            return;
        }
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
                origin_file: None,
                node_type: None,
            });
        }
        // JS/TS `export * as ns from './x'`: synthesise the `ns` binding node, a
        // file→binding `contains` edge (context `namespace_export`), and a
        // file→target-file `re_exports` edge (context `export`) (#1552). Emitted
        // here (before the id remap) so the node id and the consumer's per-file
        // `import { ns }` edge target — both `make_id([stem, ns])` — canonicalise
        // identically. `import_js` still emits the `imports_from` source edge.
        if matches!(
            config.lang_id,
            LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
        ) && t == "export_statement"
        {
            let mut nsc = node.walk();
            let children: Vec<tree_sitter::Node<'_>> = node.children(&mut nsc).collect();
            let ns_name = children
                .iter()
                .find(|ch| ch.kind() == "namespace_export")
                .and_then(|ne| {
                    let mut ic = ne.walk();
                    ne.children(&mut ic)
                        .find(|x| x.kind() == "identifier")
                        .map(|x| read_text_owned(x, source))
                });
            let src_raw = children.iter().find(|ch| ch.kind() == "string").map(|s| {
                read_text_owned(*s, source)
                    .trim_matches(|q| q == '\'' || q == '"' || q == '`' || q == ' ')
                    .to_string()
            });
            if let (Some(ns_name), Some(src_raw)) = (ns_name, src_raw)
                && !ns_name.is_empty()
                && !src_raw.is_empty()
            {
                let line = node.start_position().row as u32 + 1;
                let ns_id = make_id(&[stem, &ns_name]);
                add_node(&ns_id, &ns_name, line, str_path, ctx.nodes, ctx.seen_ids);
                add_edge(
                    file_nid,
                    &ns_id,
                    "contains",
                    line,
                    str_path,
                    Some("namespace_export"),
                    ctx.edges,
                );
                let (tgt_nid, _) = super::resolve_js_import_target(&src_raw, str_path);
                if !tgt_nid.is_empty() {
                    add_edge(
                        file_nid,
                        &tgt_nid,
                        "re_exports",
                        line,
                        str_path,
                        Some("export"),
                        ctx.edges,
                    );
                }
            }
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
        let line = node.start_position().row as u32 + 1;
        // C#: fold the enclosing namespace into the id and stamp
        // is_nested_type / namespace / scope_chain metadata (#1562). A no-op for
        // other languages, whose namespace stack is always empty.
        let class_nid = if config.lang_id == LangId::CSharp {
            let ns = ctx.csharp_ns_stack.join(".");
            let nid = make_id(&[stem, &ns, &class_name]);
            if ctx.seen_ids.insert(nid.clone()) {
                let mut pairs: Vec<(&str, Value)> = Vec::new();
                if parent_class_nid.is_some() {
                    pairs.push(("is_nested_type", Value::Bool(true)));
                }
                if !ns.is_empty() {
                    pairs.push(("namespace", Value::String(ns)));
                }
                if !ctx.csharp_scope_stack.is_empty() {
                    pairs.push((
                        "scope_chain",
                        Value::Array(
                            ctx.csharp_scope_stack
                                .iter()
                                .map(|s| Value::String(s.clone()))
                                .collect(),
                        ),
                    ));
                }
                ctx.nodes.push(GNode {
                    id: nid.clone(),
                    label: class_name.clone(),
                    file_type: "code".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    node_type: None,
                    metadata: sanitized_metadata(pairs),
                    origin_file: None,
                });
            }
            nid
        } else {
            let nid = make_id(&[stem, &class_name]);
            add_node(&nid, &class_name, line, str_path, ctx.nodes, ctx.seen_ids);
            nid
        };
        add_edge(
            file_nid, &class_nid, "contains", line, str_path, None, ctx.edges,
        );
        ctx.callable_def_nids.insert(class_nid.clone()); // a class is callable (constructor)
        // TS/JS decorators on the class and its members (@Component, @Injectable,
        // @Input, @Inject, @Entity, …) — a `references[decorator]` edge from the
        // decorated entity to the decorator symbol. Decorators live only in class
        // subtrees, so one pass over the class covers them (3540416).
        if matches!(
            config.lang_id,
            LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
        ) {
            emit_ts_decorator_edges(ctx, node, &class_nid, source);
        }

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
                                    origin_file: Some(str_path.to_string()),
                                    node_type: None,
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

        // Java / Groovy extends/implements — tree-sitter-groovy exposes the same
        // `superclass`/`interfaces` fields, so the Java path handles both (64a6093).
        if matches!(config.lang_id, LangId::Java | LangId::Groovy) {
            emit_java_inheritance(ctx, node, source, &class_nid, t, line);
            // Type-level annotations (`@Service`, `@Entity`) -> references (#1487).
            for anno_name in super::references::java_annotation_names(node, source) {
                let tgt = ensure_named_node(&anno_name, stem, str_path, ctx.nodes, ctx.seen_ids);
                if tgt != class_nid {
                    add_edge(
                        &class_nid,
                        &tgt,
                        "references",
                        line,
                        str_path,
                        Some("attribute"),
                        ctx.edges,
                    );
                }
            }
            // Java record components (the `record Order(Payload p, List<Item>
            // items)` header parameters) -> references, mirroring field types
            // (#1519). Type-parameter components are skipped by the collector.
            if t == "record_declaration" {
                emit_java_record_component_refs(ctx, node, &class_nid, source);
            }
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

        // Ruby superclass (`class Dog < Animal`)
        if config.lang_id == LangId::Ruby {
            emit_ruby_inheritance(ctx, node, source, &class_nid, line);
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
        if let Some(info) = read_csharp_type_name(type_node, source)
            && !info.name.is_empty()
            && !super::references::csharp_type_parameters_in_scope(
                type_node.unwrap_or(node),
                source,
            )
            .contains(&info.name)
        {
            let line = node.start_position().row as u32 + 1;
            let tgt = ensure_named_node(&info.name, stem, str_path, ctx.nodes, ctx.seen_ids);
            let mut pairs: Vec<(&str, Value)> =
                vec![("ref_token", Value::String(info.name.clone()))];
            if info.qualified {
                pairs.push(("qualified", Value::Bool(true)));
            }
            if !info.qualifier.is_empty() {
                pairs.push(("ref_qualifier", Value::String(info.qualifier.clone())));
            }
            ctx.edges.push(Edge {
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
                deferred: false,
                metadata: sanitized_metadata(pairs),
            });
        }
        return;
    }

    // ── C# property_declaration ───────────────────────────────────────────────
    if config.lang_id == LangId::CSharp
        && t == "property_declaration"
        && let Some(parent) = parent_class_nid
    {
        // C# auto-properties (`public Widget Main { get; set; }`) are the
        // idiomatic way to declare state, yet only field_declaration was handled.
        // A property exposes its type directly (no variable_declaration wrapper),
        // so read the `type` field and collect refs so `List<Widget>` yields both
        // the List field ref and the Widget generic_arg ref (bb5e519).
        if let Some(type_node) = node.child_by_field_name("type") {
            let line = node.start_position().row as u32 + 1;
            emit_csharp_member_type_refs(ctx, type_node, parent, line, source);
        }
        return;
    }

    // ── Java field_declaration ────────────────────────────────────────────────
    if config.lang_id == LangId::Java
        && t == "field_declaration"
        && let Some(parent) = parent_class_nid
    {
        // Field types (incl. the `generic_arg` element of `List<Handler>`) ->
        // references; primitives are skipped by `java_collect_type_refs` (#1485).
        if let Some(type_node) = node.child_by_field_name("type") {
            let line = node.start_position().row as u32 + 1;
            emit_member_type_refs(
                ctx,
                type_node,
                parent,
                line,
                source,
                super::references::java_collect_type_refs,
            );
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
        && matches!(t, "val_definition" | "var_definition")
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
                            // A method prototype (`void bar();`) is a field_declaration
                            // whose declarator is a function_declarator. Python emits it
                            // as a `defines field` member (labelled `bar`) so the impl's
                            // out-of-class `Foo::bar` definition collides on id and merges
                            // into ONE method node (#1547). Without it the header method is
                            // dropped and the def dangles off the file alone.
                            | "function_declarator"
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
                        deferred: false,
                        metadata: None,
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
        ctx.callable_def_nids.insert(func_nid.clone()); // function / method def is callable
        match config.lang_id {
            LangId::Python => {
                ctx.local_bound_names
                    .insert(func_nid.clone(), python_local_bound_names(node, source));
            }
            LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX => {
                ctx.local_bound_names
                    .insert(func_nid.clone(), js_local_bound_names(node, source));
            }
            _ => {}
        }
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

        emit_function_reference_edges(ctx, node, source, &func_nid, line, parent_class_nid);

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
                            ctx.callable_def_nids.insert(m_nid.clone());
                            ctx.local_bound_names
                                .insert(m_nid.clone(), js_local_bound_names(val, source));
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

    // ── Ruby extra walk (Struct.new/Class.new/Data.define constant classes) ────
    if config.lang_id == LangId::Ruby && super::ruby::ruby_extra_walk(ctx, node, source) {
        return;
    }

    // ── C# namespace_declaration ──────────────────────────────────────────────
    if config.lang_id == LangId::CSharp
        && matches!(
            t,
            "namespace_declaration" | "file_scoped_namespace_declaration"
        )
    {
        let ns_name = csharp_namespace_name(node, source);
        let pushed = !ns_name.is_empty();
        if pushed {
            ctx.csharp_ns_stack.push(ns_name);
            ctx.csharp_scope_stack
                .push(format!("s{}", node.start_byte()));
            let ns_label = ctx.csharp_ns_stack.join(".");
            let ns_nid = csharp_namespace_id(&ns_label);
            let line = node.start_position().row as u32 + 1;
            if ctx.seen_ids.insert(ns_nid.clone()) {
                // Canonical namespace node: `type` = "namespace", metadata carries
                // {kind, namespace}; no `scope_chain` on the namespace node itself.
                let meta = sanitized_metadata(vec![
                    ("kind", Value::String("csharp_namespace".to_string())),
                    ("namespace", Value::String(ns_label.clone())),
                ]);
                ctx.nodes.push(GNode {
                    id: ns_nid.clone(),
                    label: ns_label,
                    file_type: "code".to_string(),
                    source_file: str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    node_type: Some("namespace".to_string()),
                    metadata: meta,
                    origin_file: None,
                });
            }
            add_edge(
                file_nid, &ns_nid, "contains", line, str_path, None, ctx.edges,
            );
        }
        // A block `namespace Foo { … }` recurses its body then pops. A file-scoped
        // `namespace Foo;` has no body — it stays on the stack for the rest of the
        // file's root siblings (never popped; the walk ends), matching C# scoping.
        if t == "namespace_declaration" {
            if let Some(body) = node.child_by_field_name("body") {
                let mut cur = body.walk();
                if cur.goto_first_child() {
                    loop {
                        walk(ctx, cur.node(), parent_class_nid, source);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if pushed {
                ctx.csharp_ns_stack.pop();
                ctx.csharp_scope_stack.pop();
            }
        }
        return;
    }

    // ── TS namespace / module container (internal_module, module) ─────────────
    // `namespace Foo {}` parses as `internal_module` (name/body fields); `module
    // Bar {}` and ambient `declare module "pkg" {}` parse as a named `module` with
    // no fields, so name/body are found positionally. Emit the container node +
    // file→container `contains` edge, then recurse its body (members stay
    // file-contained, parity with the C# namespace handler). The `is_named` guard
    // skips the anonymous `module` keyword token that shares the type string.
    // Mirrors graphify-py `_ts_extra_walk` (869aaf7).
    if matches!(config.lang_id, LangId::TypeScript | LangId::TypeScriptX)
        && node.is_named()
        && matches!(t, "internal_module" | "module")
    {
        let name_node = node.child_by_field_name("name").or_else(|| {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let c = cur.node();
                    if c.is_named()
                        && matches!(c.kind(), "identifier" | "nested_identifier" | "string")
                    {
                        return Some(c);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        });
        let body = node.child_by_field_name("body").or_else(|| {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "statement_block" {
                        return Some(cur.node());
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        });
        if let Some(nn) = name_node {
            let raw = read_text_owned(nn, source);
            let ns_name = if nn.kind() == "string" {
                raw.trim_matches(|c| c == '\'' || c == '"' || c == '`')
            } else {
                &raw
            };
            if !ns_name.is_empty() {
                let ns_nid = make_id(&[stem, ns_name]);
                let line = node.start_position().row as u32 + 1;
                add_node(&ns_nid, ns_name, line, str_path, ctx.nodes, ctx.seen_ids);
                add_edge(
                    file_nid, &ns_nid, "contains", line, str_path, None, ctx.edges,
                );
            }
        }
        if let Some(body) = body {
            let mut cur = body.walk();
            if cur.goto_first_child() {
                loop {
                    walk(ctx, cur.node(), parent_class_nid, source);
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
        // Associated-value types nest as `enum_type_parameters -> user_type ->
        // type_identifier` (a sibling of the case-name); the loop above never
        // descends into them, so `case failed(Config)` used to drop the
        // NetworkError -> Config reference. Emit a `references` edge from the ENUM
        // node to each associated type (context `type`, `generic_arg` for generic
        // roles), guarding target != enum node (ad70152).
        let line = node.start_position().row as u32 + 1;
        let mut pcur = node.walk();
        if pcur.goto_first_child() {
            loop {
                let child = pcur.node();
                if child.kind() == "enum_type_parameters" {
                    for grand in named_children(child) {
                        let mut refs: Vec<(String, super::references::RefRole)> = Vec::new();
                        super::references::swift_collect_type_refs(grand, source, false, &mut refs);
                        for (name, role) in refs {
                            let target =
                                ensure_named_node(&name, stem, str_path, ctx.nodes, ctx.seen_ids);
                            if target != parent {
                                add_edge(
                                    parent,
                                    &target,
                                    "references",
                                    line,
                                    str_path,
                                    Some(role.into_context("type")),
                                    ctx.edges,
                                );
                            }
                        }
                    }
                }
                if !pcur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // ── Java enum_constant ────────────────────────────────────────────────────
    // Emit each Java enum constant as a node with a `case_of` edge to the enum,
    // and descend into an anonymous constant body (`MONDAY { void greet(){} }`)
    // so its methods attach to the constant rather than being dropped (cf36d10).
    if config.lang_id == LangId::Java
        && t == "enum_constant"
        && let Some(parent) = parent_class_nid
    {
        if let Some(name_node) = node.child_by_field_name("name") {
            let const_name = read_text_owned(name_node, source);
            let line = node.start_position().row as u32 + 1;
            let const_nid = make_id(&[parent, &const_name]);
            add_node(
                &const_nid,
                &const_name,
                line,
                str_path,
                ctx.nodes,
                ctx.seen_ids,
            );
            add_edge(
                parent, &const_nid, "case_of", line, str_path, None, ctx.edges,
            );
            // Anonymous-body constants: descend so the body's members attach to
            // the constant node rather than being dropped.
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "class_body" {
                        let mut bcur = child.walk();
                        if bcur.goto_first_child() {
                            loop {
                                walk(ctx, bcur.node(), Some(const_nid.as_str()), source);
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
        }
        return;
    }

    // ── Kotlin enum_entry ─────────────────────────────────────────────────────
    // Emit each Kotlin enum entry as a node with a `case_of` edge to the enum
    // (the `enum_class_body` fallback lets the walker descend into the enum), and
    // descend into an anonymous entry body so its members attach to the entry
    // (#1700 Kotlin half, #1738).
    if config.lang_id == LangId::Kotlin
        && t == "enum_entry"
        && let Some(parent) = parent_class_nid
    {
        let name = {
            let mut cur = node.walk();
            let mut found: Option<String> = None;
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if matches!(child.kind(), "simple_identifier" | "identifier") {
                        found = Some(read_text_owned(child, source));
                        break;
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            found
        };
        if let Some(const_name) = name {
            let line = node.start_position().row as u32 + 1;
            let const_nid = make_id(&[parent, &const_name]);
            add_node(
                &const_nid,
                &const_name,
                line,
                str_path,
                ctx.nodes,
                ctx.seen_ids,
            );
            add_edge(
                parent, &const_nid, "case_of", line, str_path, None, ctx.edges,
            );
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "class_body" {
                        let mut bcur = child.walk();
                        if bcur.goto_first_child() {
                            loop {
                                walk(ctx, bcur.node(), Some(const_nid.as_str()), source);
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
#[must_use]
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
