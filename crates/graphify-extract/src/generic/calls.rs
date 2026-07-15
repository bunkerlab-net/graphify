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

use crate::types::{Edge, RawCall, RawCallLang};

use super::config::{LangConfig, LangId};
use super::indirect::{self, IndirectState};
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
    /// Per-method `var -> ClassName` table from `var = Const.new` bindings, used
    /// to attach `receiver_type` to Ruby member-call `raw_calls` so the cross-file
    /// pass resolves `var.method` by type (#1499). Empty for non-Ruby files.
    pub ruby_var_types: &'a HashMap<String, HashMap<String, Option<String>>>,
    /// Current method body's `receiver -> type` table for Java (current-class
    /// fields plus method parameters and explicit locals, with `this.field`
    /// entries). Lets member-call `raw_calls` carry a `receiver_type` for
    /// type-based cross-file resolution (#1696). Empty for non-Java files.
    pub java_var_types: &'a HashMap<String, String>,
    /// File-wide `name -> Type` table for C# (fields / properties / params /
    /// locals). Lets member-call `raw_calls` carry a `receiver_type` for
    /// type-based cross-file resolution (#1609). Empty for non-C# files.
    pub csharp_var_types: &'a HashMap<String, String>,
    /// Current function body's `var -> ClassName` table for C++ (its local
    /// declarations). Lets member-call `raw_calls` carry a `receiver_type` for
    /// type-based cross-file resolution (#1547). Empty for non-C++ files.
    pub cpp_var_types: &'a HashMap<String, String>,
    /// File-wide `name -> TypeName` table for TS/JS (constructor-injected
    /// `this.field` types, local `new` bindings, typed params). Lets member-call
    /// `raw_calls` carry a `receiver_type` for cross-file resolution (#1316/#1630).
    /// Empty for non-TS/JS files.
    pub ts_var_types: &'a HashMap<String, String>,
    /// Node ids of the bodies already walked with their own caller (const-assigned
    /// arrows, methods). A JS/TS inline/returned closure NOT in this set is
    /// descended with the enclosing caller so its calls aren't lost at the arrow
    /// boundary (#1630). Empty for non-TS/JS files.
    pub tracked_body_ids: &'a HashSet<usize>,
    /// Case-sensitive `label -> nid` map for indirect refs (the call map above is
    /// lowercased). Preserves case-sensitivity hardening (#1581).
    pub label_to_nid_exact: &'a HashMap<String, String>,
    /// `nid -> source_file`, so the indirect guard tells a same-named local
    /// non-callable (reject) from an import-surfaced foreign symbol (defer).
    pub nid_to_sf: &'a HashMap<String, String>,
    /// Ids of function / method / class definitions in this file — the callable
    /// targets an `indirect_call` reference may resolve to (#1565/#1566).
    pub callable_def_nids: &'a HashSet<String>,
    /// Python / JS-TS per-function local-binding shadow sets, keyed by caller nid.
    pub local_bound_names: &'a HashMap<String, HashSet<String>>,
    /// Dedup for emitted `indirect_call` pairs.
    pub seen_indirect_pairs: &'a mut HashSet<(String, String)>,
}

impl CallWalkCtx<'_> {
    /// Bundle the disjoint indirect-dispatch fields into an [`IndirectState`] for a
    /// capture site. Fetch `enclosing_locals` from `local_bound_names` BEFORE
    /// calling this (it mutably reborrows `self`).
    fn indirect(&mut self) -> IndirectState<'_> {
        IndirectState {
            str_path: self.str_path,
            label_to_nid_exact: self.label_to_nid_exact,
            nid_to_sf: self.nid_to_sf,
            callable_def_nids: self.callable_def_nids,
            edges: &mut *self.edges,
            raw_calls: &mut *self.raw_calls,
            seen_call_pairs: &*self.seen_call_pairs,
            seen_indirect_pairs: &mut *self.seen_indirect_pairs,
        }
    }
}

/// The declared type of a member call's receiver, for the languages that carry a
/// type table: Ruby (`var = Const.new` inference, #1499), Java (fields / params /
/// locals, #1696), C# (file-wide members, #1609), and C++ (local declarations,
/// #1547). `None` for other languages, unknown, or ambiguous receivers.
fn member_receiver_type(
    ctx: &CallWalkCtx<'_>,
    caller_nid: &str,
    receiver: Option<&str>,
) -> Option<String> {
    let receiver = receiver?;
    match ctx.config.lang_id {
        LangId::Ruby => ctx
            .ruby_var_types
            .get(caller_nid)
            .and_then(|m| m.get(receiver).cloned())
            .flatten(),
        LangId::Java => ctx.java_var_types.get(receiver).cloned(),
        LangId::CSharp => ctx.csharp_var_types.get(receiver).cloned(),
        LangId::Cpp => ctx.cpp_var_types.get(receiver).cloned(),
        LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX => {
            ctx.ts_var_types.get(receiver).cloned()
        }
        _ => None,
    }
}

/// Stops descending into nested function definitions (`function_boundary_types`)
/// so that calls from inner lambdas are attributed to the outer function.
/// Mirrors Python `_walk_calls` from `extract.py`.
#[allow(clippy::too_many_lines)] // linear boundary/call/recurse dispatch + JS/TS closure descent
pub(super) fn walk_calls(
    ctx: &mut CallWalkCtx<'_>,
    node: Node<'_>,
    caller_nid: &str,
    source: &[u8],
) {
    if ctx.config.function_boundary_types.contains(&node.kind()) {
        // JS/TS: an inline/returned closure (`return () => svc.doThing()`) is a
        // boundary but is NOT separately tracked in `function_bodies`, so its
        // calls would be dropped here. Descend into it with the enclosing caller;
        // a tracked closure (const-assigned arrow) is walked with its own nid and
        // skipped to avoid double-counting (#1630).
        if matches!(
            ctx.config.lang_id,
            LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
        ) && matches!(node.kind(), "arrow_function" | "function_expression")
            && let Some(body) = node.child_by_field_name("body")
            && !ctx.tracked_body_ids.contains(&body.id())
        {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_calls(ctx, cur.node(), caller_nid, source);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
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

        let (callee_name, is_member_call, receiver, is_this_field) =
            extract_callee(node, source, ctx.config, ctx.label_to_nid);

        if let Some(callee) = callee_name
            && !callee.is_empty()
        {
            // Resolve first: a built-in name (`len`, `String`, ...) that maps to a
            // real local symbol is a genuine call and must be kept. Only drop
            // built-ins when they DON'T resolve, so they can't become cross-file
            // god-nodes via the raw-call pass (#726).
            // Ruby: the receiver's inferred type from the method's local
            // `var = Const.new` bindings, when unambiguously known (#1499).
            // Computed up-front so it can also gate member-call deferral below.
            let receiver_type = member_receiver_type(ctx, caller_nid, receiver.as_deref());
            let receiver_upper = receiver
                .as_deref()
                .is_some_and(|r| r.chars().next().is_some_and(char::is_uppercase));
            // Defer to receiver-based cross-file resolution rather than a bare
            // same-file `label_to_nid` match (which can collide with an in-file
            // symbol — even the caller — and drop or mis-link the call):
            //   * Python `ClassName.method()` (upper-cased receiver), #1446;
            //   * Ruby `Const.new` / `Const.method()` (upper-cased receiver), and a
            //     typed instance call `var.method()` whose `var` has a known type
            //     (#1499). graphify-py only defers upper-cased receivers
            //     (extract.py:3899); deferring the typed-instance case too keeps a
            //     `p.run` from being swallowed by a same-file `run`.
            //   * Java: EVERY member call defers; `resolve_java_member_calls` binds
            //     it by the receiver's declared type, never a bare name (#1696).
            //   * C#: EVERY member call with a captured receiver defers to
            //     `resolve_csharp_member_calls`, never a bare name (#1609).
            let defer_member = is_member_call
                && ((ctx.config.lang_id == LangId::Python && receiver_upper)
                    || (ctx.config.lang_id == LangId::Ruby
                        && (receiver_upper || receiver_type.is_some()))
                    || ctx.config.lang_id == LangId::Java
                    || ctx.config.lang_id == LangId::Cpp
                    || (ctx.config.lang_id == LangId::CSharp && receiver.is_some())
                    // JS/TS: defer a `ClassName.method()` (upper) or a
                    // `this.field.method()` (constructor-injected type) call so the
                    // cross-file resolver binds it by the receiver's type (#1316).
                    || (matches!(
                        ctx.config.lang_id,
                        LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
                    ) && (receiver_upper || is_this_field)));
            let tgt_nid = if defer_member {
                None
            } else {
                ctx.label_to_nid.get(&callee.to_lowercase()).cloned()
            };
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
                            deferred: false,
                            metadata: None,
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
                    receiver_type,
                    lang: (ctx.config.lang_id == LangId::Cpp).then_some(RawCallLang::Cpp),
                    ..Default::default()
                });
            }

            emit_php_call_relations(ctx, node, &callee, caller_nid, source);
        }
        capture_call_indirect(ctx, node, caller_nid, source);
    }

    emit_php_ref_relations(ctx, node, caller_nid, source);
    capture_node_indirect(ctx, node, caller_nid, source);

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
) -> (Option<String>, bool, Option<String>, bool) {
    let mut callee_name: Option<String> = None;
    let mut is_member_call = false;
    let mut receiver: Option<String> = None;
    // JS/TS `this.field.method()`: the field-name receiver must always defer to
    // the constructor-injection resolver, even when a same-file method matches.
    let mut is_this_field = false;

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
            // The invoked function is the `function` field. A member call
            // `recv.Method(...)` is a member_access_expression (receiver in
            // `expression`, method in `name`). Capture a simple-identifier or
            // `this` receiver + set is_member_call so `resolve_csharp_member_calls`
            // binds by the receiver's declared type; a bare name mis-bound to any
            // same-named method in the corpus (#1609).
            match node.child_by_field_name("function").map(|f| (f, f.kind())) {
                Some((fnn, "member_access_expression")) => {
                    if let Some(mname) = fnn.child_by_field_name("name") {
                        callee_name = Some(read_text_owned(mname, source));
                        is_member_call = true;
                        match fnn.child_by_field_name("expression").map(|r| (r, r.kind())) {
                            Some((recv, "identifier")) => {
                                receiver = Some(read_text_owned(recv, source));
                            }
                            Some((_, "this_expression")) => receiver = Some("this".to_string()),
                            _ => {}
                        }
                    }
                }
                Some((fnn, "identifier")) => callee_name = Some(read_text_owned(fnn, source)),
                _ => {
                    // Fallback: `name` field, else first-named-child dotted scan.
                    if let Some(name_node) = node.child_by_field_name("name") {
                        callee_name = Some(read_text_owned(name_node, source));
                    } else {
                        let mut cur = node.walk();
                        if cur.goto_first_child() {
                            loop {
                                let child = cur.node();
                                if child.is_named() {
                                    let raw = read_text_owned(child, source);
                                    if let Some((head, tail)) = raw.rsplit_once('.') {
                                        callee_name = Some(tail.to_string());
                                        is_member_call = true;
                                        if !head.is_empty() && !head.contains('.') {
                                            receiver = Some(head.to_string());
                                        }
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
                } else if func_node.kind() == "field_expression" {
                    // `f.bar()` / `f->bar()` / `this->bar()`: receiver is the
                    // `argument` (object) field, callee is the `field` (#1547).
                    // Capture a simple-identifier or `this` receiver so the
                    // cross-file pass can type it; a chained receiver (`a.b.m()`)
                    // stays unset and the call is left to bail.
                    is_member_call = true;
                    if let Some(n) = func_node.child_by_field_name("field") {
                        callee_name = Some(read_text_owned(n, source));
                    }
                    match func_node.child_by_field_name("argument") {
                        Some(obj) if obj.kind() == "identifier" => {
                            receiver = Some(read_text_owned(obj, source));
                        }
                        Some(obj) if obj.kind() == "this" => {
                            receiver = Some("this".to_string());
                        }
                        _ => {}
                    }
                } else if func_node.kind() == "qualified_identifier" {
                    // `Foo::bar()`: the scope `Foo` names the receiver type
                    // explicitly (EXTRACTED), the `name` is the callee.
                    is_member_call = true;
                    if let Some(n) = func_node.child_by_field_name("name") {
                        callee_name = Some(read_text_owned(n, source));
                    }
                    if let Some(scope) = func_node.child_by_field_name("scope") {
                        receiver = Some(read_text_owned(scope, source));
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
        LangId::Java if node.kind() == "method_invocation" => {
            // `recv.method(args)` — `name` is the method, `object` the receiver.
            // Capture a simple / `this` / `this.field` receiver so cross-file
            // member-call resolution can bind by the receiver's declared type
            // (#1696). A chained/qualified receiver stays unset (deferred).
            if let Some(name_node) = node.child_by_field_name("name") {
                callee_name = Some(read_text_owned(name_node, source));
            }
            if let Some(recv) = node.child_by_field_name("object") {
                is_member_call = true;
                match recv.kind() {
                    "identifier" => receiver = Some(read_text_owned(recv, source)),
                    "this" => receiver = Some("this".to_string()),
                    "field_access" => {
                        if let (Some(owner), Some(field)) = (
                            recv.child_by_field_name("object"),
                            recv.child_by_field_name("field"),
                        ) && owner.kind() == "this"
                        {
                            receiver = Some(format!("this.{}", read_text_owned(field, source)));
                        }
                    }
                    _ => {}
                }
            }
        }
        LangId::Ruby => {
            // Ruby's `call` node carries `receiver` and `method` as direct fields
            // (no intermediate accessor node), so the generic accessor model
            // doesn't apply. Read them directly and capture a simple receiver (`p`
            // in `p.run`, `Processor` in `Processor.new`) so the cross-file pass
            // can resolve member calls by the receiver's type (#1499).
            if let Some(meth) = node.child_by_field_name("method") {
                callee_name = Some(read_text_owned(meth, source));
            }
            if let Some(recv) = node.child_by_field_name("receiver") {
                is_member_call = true;
                if matches!(recv.kind(), "identifier" | "constant") {
                    receiver = Some(read_text_owned(recv, source));
                } else if recv.kind() == "scope_resolution" {
                    // Namespaced receiver `Billing::Processor.call` — capture the last
                    // constant so cross-file resolution binds it by the bare class name
                    // (the god-node guard bails if ambiguous, #1634).
                    let last = super::ruby::ruby_const_last_name(recv, source);
                    if !last.is_empty() {
                        receiver = Some(last);
                    }
                }
            }
        }
        LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX => {
            if let Some(func_node) = node.child_by_field_name("function") {
                if func_node.kind() == "identifier" {
                    callee_name = Some(read_text_owned(func_node, source));
                } else if func_node.kind() == "member_expression" {
                    is_member_call = true;
                    if let Some(prop) = func_node.child_by_field_name("property") {
                        callee_name = Some(read_text_owned(prop, source));
                    }
                    // `ClassName.method()` -> simple-identifier receiver (#1446).
                    // `this.field.method()` -> the field name + a flag so the
                    // constructor-injection type table resolves it (#1316). Deeper
                    // chains (`a.b.method()`) stay unset.
                    match func_node.child_by_field_name("object") {
                        Some(obj) if obj.kind() == "identifier" => {
                            receiver = Some(read_text_owned(obj, source));
                        }
                        Some(obj) if obj.kind() == "member_expression" => {
                            if let Some(inner_obj) = obj.child_by_field_name("object")
                                && inner_obj.kind() == "this"
                                && let Some(inner_prop) = obj.child_by_field_name("property")
                            {
                                receiver = Some(read_text_owned(inner_prop, source));
                                is_this_field = true;
                            }
                        }
                        _ => {}
                    }
                } else {
                    callee_name = Some(read_text_owned(func_node, source));
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
                    // #1446: capture a simple-identifier receiver (`ClassName` in
                    // `ClassName.method()`) so cross-file resolution can resolve
                    // qualified Python class-method calls. Chained receivers
                    // (`a.b.method()`) are skipped.
                    if config.lang_id == LangId::Python
                        && let Some(obj) = func_node.child_by_field_name("object")
                        && obj.kind() == "identifier"
                    {
                        receiver = Some(read_text_owned(obj, source));
                    }
                } else {
                    callee_name = Some(read_text_owned(func_node, source));
                }
            }
        }
    }

    (callee_name, is_member_call, receiver, is_this_field)
}

/// First string-literal argument of a PHP call's `arguments` (e.g. `'foo.bar'`
/// in `config('foo.bar')`). Mirrors graphify-py's helper-fn-call arg scan.
#[must_use]
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
#[must_use]
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
#[must_use]
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
        deferred: false,
        metadata: None,
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

// ── Indirect-dispatch capture (#1565/#1566) ───────────────────────────────────

/// Capture `indirect_call` references from a CALL node: identifier / keyword
/// arguments passed by name, plus Python `getattr(obj, "name")` reflective
/// dispatch. `node` is the call expression; `caller_nid` the enclosing function.
fn capture_call_indirect(
    ctx: &mut CallWalkCtx<'_>,
    node: Node<'_>,
    caller_nid: &str,
    source: &[u8],
) {
    let is_python = ctx.config.lang_id == LangId::Python;
    let is_js_ts = matches!(
        ctx.config.lang_id,
        LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
    );
    if !is_python && !is_js_ts {
        return;
    }
    let empty = HashSet::new();
    let enclosing = ctx.local_bound_names.get(caller_nid).unwrap_or(&empty);
    if let Some(args) = node.child_by_field_name("arguments") {
        let mut cur = args.walk();
        for arg in args.children(&mut cur) {
            match arg.kind() {
                "identifier" => {
                    ctx.indirect()
                        .emit_ref(Some(arg), caller_nid, enclosing, "argument", source);
                }
                // Python keyword arg `target=fn`; JS has no keyword args (named
                // args are objects, handled by the collection pass).
                "keyword_argument" if is_python => {
                    ctx.indirect().emit_ref(
                        arg.child_by_field_name("value"),
                        caller_nid,
                        enclosing,
                        "argument",
                        source,
                    );
                }
                _ => {}
            }
        }
    }
    // Reflective dispatch: `getattr(obj, "handler")` — the string is an ATTRIBUTE
    // name, never shadowed by a param/local, so it bypasses the identifier guard.
    if is_python && let Some((name, loc)) = indirect::getattr_ref_name(node, source) {
        ctx.indirect()
            .emit_by_name(&name, loc, caller_nid, "getattr");
    }
}

/// Capture `indirect_call` references from a non-call node inside a function body:
/// dispatch-table values (dict/list/set/tuple or object/array), Python assignment
/// RHS, and Python `return` values. Attributed to the enclosing `caller_nid`.
fn capture_node_indirect(
    ctx: &mut CallWalkCtx<'_>,
    node: Node<'_>,
    caller_nid: &str,
    source: &[u8],
) {
    let empty = HashSet::new();
    let enclosing = ctx.local_bound_names.get(caller_nid).unwrap_or(&empty);
    match ctx.config.lang_id {
        LangId::Python => match node.kind() {
            "dictionary" | "list" | "set" | "tuple" => {
                for ident in indirect::python_dispatch_value_idents(node) {
                    ctx.indirect().emit_ref(
                        Some(ident),
                        caller_nid,
                        enclosing,
                        "collection",
                        source,
                    );
                }
            }
            "assignment" => {
                for ident in indirect::python_ref_value_idents(node.child_by_field_name("right")) {
                    ctx.indirect().emit_ref(
                        Some(ident),
                        caller_nid,
                        enclosing,
                        "assignment",
                        source,
                    );
                }
            }
            "return_statement" => {
                let value = {
                    let mut cur = node.walk();
                    node.children(&mut cur).find(tree_sitter::Node::is_named)
                };
                for ident in indirect::python_ref_value_idents(value) {
                    ctx.indirect()
                        .emit_ref(Some(ident), caller_nid, enclosing, "return", source);
                }
            }
            _ => {}
        },
        LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
            if matches!(node.kind(), "object" | "array") =>
        {
            for ident in indirect::js_dispatch_value_idents(node) {
                ctx.indirect()
                    .emit_ref(Some(ident), caller_nid, enclosing, "collection", source);
            }
        }
        _ => {}
    }
}
