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

        let (callee_name, is_member_call) =
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
                });
            }
        }
    }

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
) -> (Option<String>, bool) {
    let mut callee_name: Option<String> = None;
    let mut is_member_call = false;

    match config.lang_id {
        LangId::Swift => {
            if let Some(first) = node.child(0) {
                if first.kind() == "simple_identifier" {
                    callee_name = Some(read_text_owned(first, source));
                } else if first.kind() == "navigation_expression" {
                    is_member_call = true;
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

    (callee_name, is_member_call)
}
