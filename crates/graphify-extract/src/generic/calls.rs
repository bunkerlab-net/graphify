//! Call-graph walk and per-language callee extraction.
//!
//! `walk_calls` descends into a function body's AST collecting `calls` edges
//! and unresolved `RawCall` entries.  `extract_callee` is the per-language
//! dispatch that knows how to find the callee name from each language's call
//! expression node shape.

// Tree-sitter row numbers are source line indices; files with 2^32+ lines
// do not exist in practice, so usize→u32 truncation is safe.
#![allow(clippy::cast_possible_truncation)]

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::types::{Edge, RawCall};

use super::config::{LangConfig, LangId};
use super::js_extra::dynamic_import_js;
use super::names::read_text_owned;
use super::walk::{named_children, php_class_const_scope};

// ── Call-graph walk ───────────────────────────────────────────────────────────

/// Recursively collect call edges from a function body node.
///
/// Shared state threaded through every [`walk_calls`] recursion.
pub(super) struct CallWalkCtx<'a> {
    pub config: &'a LangConfig,
    pub str_path: &'a str,
    pub label_to_nid: &'a HashMap<String, String>,
    pub seen_call_pairs: &'a mut HashSet<(String, String)>,
    pub seen_dyn_import_pairs: &'a mut HashSet<(String, String)>,
    pub edges: &'a mut Vec<Edge>,
    pub raw_calls: &'a mut Vec<RawCall>,
    pub seen_ref_pairs: &'a mut HashSet<(String, String, String)>,
}

/// Stops descending into nested function definitions (`function_boundary_types`)
/// so that calls from inner lambdas are attributed to the outer function.
/// Mirrors Python `_walk_calls` from `extract.py`.
pub(super) fn walk_calls(
    ctx: &mut CallWalkCtx<'_>,
    node: Node<'_>,
    caller_nid: &str,
    source: &[u8],
) {
    if ctx.config.function_boundary_types.contains(&node.kind()) {
        return;
    }

    if ctx.config.call_types.contains(&node.kind()) {
        // JS/TS: detect dynamic import() calls
        if (ctx.config.lang_id == LangId::JavaScript
            || ctx.config.lang_id == LangId::TypeScript
            || ctx.config.lang_id == LangId::TypeScriptX)
            && dynamic_import_js(
                node,
                source,
                caller_nid,
                ctx.str_path,
                ctx.edges,
                ctx.seen_dyn_import_pairs,
            )
        {
            // Still recurse
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    walk_calls(ctx, child, caller_nid, source);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            return;
        }

        let (callee_name, is_member_call, receiver) =
            extract_callee(node, source, ctx.config, ctx.label_to_nid);

        if let Some(callee) = callee_name
            && !callee.is_empty()
        {
            // Resolve first: a built-in name (`len`, `String`, ...) that maps to a
            // real local symbol is a genuine call and must be kept. Only drop
            // built-ins when they DON'T resolve, so they can't become cross-file
            // god-nodes via the raw-call pass (#726).
            let tgt_nid = ctx.label_to_nid.get(&callee.to_lowercase()).cloned();
            if let Some(tgt) = tgt_nid {
                if tgt != caller_nid {
                    let pair = (caller_nid.to_string(), tgt.clone());
                    if ctx.seen_call_pairs.insert(pair) {
                        let line = node.start_position().row as u32 + 1;
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
            } else if !crate::builtins::is_language_builtin_global(&callee) {
                ctx.raw_calls.push(RawCall {
                    caller_nid: caller_nid.to_string(),
                    callee: callee.clone(),
                    is_member_call,
                    source_file: ctx.str_path.to_string(),
                    source_location: format!("L{}", node.start_position().row + 1),
                    receiver,
                });
            }

            emit_php_call_relations(ctx, node, &callee, caller_nid, source);
        }
    }

    emit_php_ref_relations(ctx, node, caller_nid, source);

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            walk_calls(ctx, child, caller_nid, source);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Return the depth-1 receiver name of a Swift member call (`recv.method()`):
/// `vm.update()` -> `vm`; `Type.staticMethod()` -> `Type`;
/// `Singleton.shared.method()` -> `Singleton`; `self.svc.fetch()` -> `svc`.
/// `None` for anything deeper, keeping resolution depth-1. Mirrors
/// `_swift_receiver_name`.
fn swift_receiver_name(recv: Node<'_>, source: &[u8]) -> Option<String> {
    match recv.kind() {
        "simple_identifier" => Some(read_text_owned(recv, source)),
        "navigation_expression" => {
            let head = recv.child(0)?;
            if head.kind() == "simple_identifier" {
                return Some(read_text_owned(head, source));
            }
            if head.kind() == "self_expression" {
                let mut cur = recv.walk();
                if cur.goto_first_child() {
                    loop {
                        let child = cur.node();
                        if child.kind() == "navigation_suffix" {
                            let mut sc = child.walk();
                            if sc.goto_first_child() {
                                loop {
                                    if sc.node().kind() == "simple_identifier" {
                                        return Some(read_text_owned(sc.node(), source));
                                    }
                                    if !sc.goto_next_sibling() {
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
            None
        }
        _ => None,
    }
}

// ── Callee extraction ─────────────────────────────────────────────────────────

/// Extract the callee name and member-call flag from a call expression node.
///
/// Each language stores the callee differently in its AST, so this function
/// dispatches per `LangId`. Returns `(Option<callee_name>, is_member_call)`.
/// `is_member_call` is `true` when the call is on an object (e.g. `obj.method()`)
/// so the cross-file resolver can skip it — member-call resolution would require
/// type inference that the extractor does not perform.
#[allow(clippy::too_many_lines)]
fn extract_callee(
    node: Node<'_>,
    source: &[u8],
    config: &LangConfig,
    _label_to_nid: &HashMap<String, String>,
) -> (Option<String>, bool, Option<String>) {
    let mut callee_name: Option<String> = None;
    let mut is_member_call = false;
    let mut receiver: Option<String> = None;

    match config.lang_id {
        LangId::Swift => {
            if let Some(first) = node.child(0) {
                if first.kind() == "simple_identifier" {
                    callee_name = Some(read_text_owned(first, source));
                } else if first.kind() == "navigation_expression" {
                    is_member_call = true;
                    // #1356: capture the depth-1 receiver so cross-file
                    // member-call resolution can type it via the file's table.
                    receiver = first
                        .child(0)
                        .and_then(|recv| swift_receiver_name(recv, source));
                    let mut cur = first.walk();
                    if cur.goto_first_child() {
                        loop {
                            let child = cur.node();
                            if child.kind() == "navigation_suffix" {
                                let mut scur = child.walk();
                                if scur.goto_first_child() {
                                    loop {
                                        let sc = scur.node();
                                        if sc.kind() == "simple_identifier" {
                                            callee_name = Some(read_text_owned(sc, source));
                                        }
                                        if !scur.goto_next_sibling() {
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
            }
        }
        LangId::Kotlin => {
            if let Some(first) = node.child(0) {
                if matches!(first.kind(), "simple_identifier" | "identifier") {
                    callee_name = Some(read_text_owned(first, source));
                } else if first.kind() == "navigation_expression" {
                    is_member_call = true;
                    // Reversed scan for last simple_identifier. tree-sitter 0.26
                    // moved `Node::child` to take `u32`; cast inside the loop.
                    let count = u32::try_from(first.child_count()).unwrap_or(0);
                    for i in (0..count).rev() {
                        if let Some(c) = first.child(i)
                            && matches!(c.kind(), "simple_identifier" | "identifier")
                        {
                            callee_name = Some(read_text_owned(c, source));
                            break;
                        }
                    }
                }
            }
        }
        LangId::Scala => {
            if let Some(first) = node.child(0) {
                if first.kind() == "identifier" {
                    callee_name = Some(read_text_owned(first, source));
                } else if first.kind() == "field_expression" {
                    is_member_call = true;
                    if let Some(field) = first.child_by_field_name("field") {
                        callee_name = Some(read_text_owned(field, source));
                    } else {
                        let count = u32::try_from(first.child_count()).unwrap_or(0);
                        for i in (0..count).rev() {
                            if let Some(c) = first.child(i)
                                && c.kind() == "identifier"
                            {
                                callee_name = Some(read_text_owned(c, source));
                                break;
                            }
                        }
                    }
                }
            }
        }
        LangId::CSharp if node.kind() == "invocation_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                callee_name = Some(read_text_owned(name_node, source));
            } else {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        let child = cur.node();
                        if child.is_named() {
                            let raw = read_text_owned(child, source);
                            if raw.contains('.') {
                                callee_name =
                                    Some(raw.split('.').next_back().unwrap_or("").to_string());
                                is_member_call = true;
                            } else {
                                callee_name = Some(raw);
                            }
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        LangId::Php => match node.kind() {
            "function_call_expression" => {
                if let Some(f) = node.child_by_field_name("function") {
                    callee_name = Some(read_text_owned(f, source));
                }
            }
            "scoped_call_expression" => {
                if let Some(scope) = node.child_by_field_name("scope") {
                    callee_name = Some(read_text_owned(scope, source));
                }
            }
            _ => {
                is_member_call = true;
                if let Some(name) = node.child_by_field_name("name") {
                    callee_name = Some(read_text_owned(name, source));
                }
            }
        },
        LangId::Cpp => {
            if !config.call_function_field.is_empty()
                && let Some(func_node) = node.child_by_field_name(config.call_function_field)
            {
                if func_node.kind() == "identifier" {
                    callee_name = Some(read_text_owned(func_node, source));
                } else if matches!(
                    func_node.kind(),
                    "field_expression" | "qualified_identifier"
                ) {
                    is_member_call = true;
                    let name = func_node
                        .child_by_field_name("field")
                        .or_else(|| func_node.child_by_field_name("name"));
                    if let Some(n) = name {
                        callee_name = Some(read_text_owned(n, source));
                    }
                }
            }
        }
        LangId::Java if node.kind() == "object_creation_expression" => {
            // `new Foo(...)` — the constructed type is in the `type` field, not
            // `name`, so the generic path misses it (#1373). Reduce a qualified /
            // generic type to its simple name (`com.a.Foo<Bar>` -> `Foo`). Java
            // `method_invocation` still flows through the generic branch below.
            if let Some(type_node) = node.child_by_field_name("type") {
                let raw = read_text_owned(type_node, source);
                let simple = raw.split('<').next().unwrap_or("").trim();
                if !simple.is_empty() {
                    callee_name = Some(simple.rsplit('.').next().unwrap_or(simple).to_string());
                }
            }
        }
        _ => {
            // Generic: use call_function_field
            if !config.call_function_field.is_empty()
                && let Some(func_node) = node.child_by_field_name(config.call_function_field)
            {
                if func_node.kind() == "identifier" {
                    callee_name = Some(read_text_owned(func_node, source));
                } else if config.call_accessor_node_types.contains(&func_node.kind()) {
                    is_member_call = true;
                    if !config.call_accessor_field.is_empty()
                        && let Some(attr) =
                            func_node.child_by_field_name(config.call_accessor_field)
                    {
                        callee_name = Some(read_text_owned(attr, source));
                    }
                } else {
                    callee_name = Some(read_text_owned(func_node, source));
                }
            }
        }
    }

    (callee_name, is_member_call, receiver)
}

/// First string-literal argument of a PHP call's `arguments` (e.g. `'foo.bar'`
/// in `config('foo.bar')`). Mirrors graphify-py's helper-fn-call arg scan.
fn php_first_string_arg(node: Node<'_>, source: &[u8]) -> Option<String> {
    let args = node.child_by_field_name("arguments")?;
    for arg in named_children(args) {
        if arg.kind() != "argument" {
            continue;
        }
        for inner in named_children(arg) {
            if inner.kind() == "string"
                && let Some(sc) = named_children(inner)
                    .into_iter()
                    .find(|c| c.kind() == "string_content")
            {
                return Some(read_text_owned(sc, source));
            }
        }
    }
    None
}

/// Up to two `Foo::class` arguments of a PHP container-bind call
/// (`$app->bind(Foo::class, Bar::class)`). Mirrors graphify-py's bind arg scan.
fn php_bind_class_args(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut classes: Vec<String> = Vec::new();
    let Some(args) = node.child_by_field_name("arguments") else {
        return classes;
    };
    for arg in named_children(args) {
        if arg.kind() != "argument" {
            continue;
        }
        if let Some(cc) = named_children(arg)
            .into_iter()
            .find(|c| c.kind() == "class_constant_access_expression")
            && let Some(cls) = php_class_const_scope(cc, source)
        {
            classes.push(cls);
        }
        if classes.len() >= 2 {
            break;
        }
    }
    classes
}

/// Build an EXTRACTED PHP reference edge (used for `uses_config`, `bound_to`,
/// `uses_static_prop`, and `references_constant`).
fn php_ref_edge(src: &str, tgt: &str, relation: &str, line: u32, str_path: &str) -> Edge {
    Edge {
        external: false,
        source: src.to_string(),
        target: tgt.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: None,
        confidence_score: Some(1.0),
    }
}

/// Emit PHP call-site reference edges (`uses_config`, `bound_to`) for a call
/// node whose resolved callee is `callee`. No-op for non-PHP configs (the
/// relevant config lists are empty). Mirrors graphify-py's helper-fn and
/// container-bind branches in `walk_calls`.
fn emit_php_call_relations(
    ctx: &mut CallWalkCtx<'_>,
    node: Node<'_>,
    callee: &str,
    caller_nid: &str,
    source: &[u8],
) {
    // config('foo.bar') -> uses_config edge to "foo".
    if ctx.config.helper_fn_names.contains(&callee)
        && let Some(first_key) = php_first_string_arg(node, source)
    {
        let segment = first_key.split('.').next().unwrap_or("").to_lowercase();
        let tgt = ctx
            .label_to_nid
            .get(&segment)
            .or_else(|| ctx.label_to_nid.get(&format!("{segment}.php")))
            .cloned();
        if let Some(tgt) = tgt
            && tgt != caller_nid
        {
            let relation = format!("uses_{callee}");
            if ctx
                .seen_ref_pairs
                .insert((caller_nid.to_string(), tgt.clone(), relation.clone()))
            {
                let line = node.start_position().row as u32 + 1;
                ctx.edges.push(php_ref_edge(
                    caller_nid,
                    &tgt,
                    &relation,
                    line,
                    ctx.str_path,
                ));
            }
        }
    }

    // $app->bind(Foo::class, Bar::class) -> bound_to edge.
    if node.kind() == "member_call_expression"
        && ctx.config.container_bind_methods.contains(&callee)
    {
        let classes = php_bind_class_args(node, source);
        if let [contract, impl_] = classes.as_slice() {
            let contract_nid = ctx.label_to_nid.get(&contract.to_lowercase()).cloned();
            let impl_nid = ctx.label_to_nid.get(&impl_.to_lowercase()).cloned();
            if let (Some(contract_nid), Some(impl_nid)) = (contract_nid, impl_nid)
                && contract_nid != impl_nid
                && ctx.seen_ref_pairs.insert((
                    contract_nid.clone(),
                    impl_nid.clone(),
                    "bound_to".to_string(),
                ))
            {
                let line = node.start_position().row as u32 + 1;
                ctx.edges.push(php_ref_edge(
                    &contract_nid,
                    &impl_nid,
                    "bound_to",
                    line,
                    ctx.str_path,
                ));
            }
        }
    }
}

/// Emit PHP reference edges from non-call nodes: `uses_static_prop` (`Foo::$bar`)
/// and `references_constant` (`Foo::BAR`). No-op for non-PHP configs.
fn emit_php_ref_relations(
    ctx: &mut CallWalkCtx<'_>,
    node: Node<'_>,
    caller_nid: &str,
    source: &[u8],
) {
    if ctx.config.static_prop_types.contains(&node.kind())
        && let Some(class_name) = php_class_const_scope(node, source)
        && let Some(tgt) = ctx.label_to_nid.get(&class_name.to_lowercase()).cloned()
        && tgt != caller_nid
        && ctx.seen_ref_pairs.insert((
            caller_nid.to_string(),
            tgt.clone(),
            "uses_static_prop".to_string(),
        ))
    {
        let line = node.start_position().row as u32 + 1;
        ctx.edges.push(php_ref_edge(
            caller_nid,
            &tgt,
            "uses_static_prop",
            line,
            ctx.str_path,
        ));
    }

    if ctx.config.lang_id == LangId::Php
        && node.kind() == "class_constant_access_expression"
        && let Some(class_name) = php_class_const_scope(node, source)
        && let Some(tgt) = ctx.label_to_nid.get(&class_name.to_lowercase()).cloned()
        && tgt != caller_nid
        && ctx.seen_ref_pairs.insert((
            caller_nid.to_string(),
            tgt.clone(),
            "references_constant".to_string(),
        ))
    {
        let line = node.start_position().row as u32 + 1;
        ctx.edges.push(php_ref_edge(
            caller_nid,
            &tgt,
            "references_constant",
            line,
            ctx.str_path,
        ));
    }
}
