//! Indirect-dispatch (`indirect_call`) capture: a function referenced BY NAME —
//! passed as a call argument (`pool.submit(fn)`), listed as a value in a dispatch
//! table (`{"k": fn}`), bound (`cb = fn`), returned (`return fn`), or named by a
//! `getattr(obj, "fn")` string literal — is a real dependency the callee-only
//! call scan cannot see. Captured as a distinct INFERRED `indirect_call` edge so
//! the strict `calls` relation stays precise while `affected` picks it up.
//!
//! Two soundness guards keep it from manufacturing false edges: an argument that
//! is a parameter / local binding names a local value (SHADOWING), and the target
//! must resolve to a real callable definition (CALLABLE TARGET). A name defined in
//! another file is deferred to the cross-file resolver via an `indirect` `raw_call`.
//!
//! Ports the `_emit_indirect_*` / shadow-name helpers from graphify-py's
//! `extractors/engine.py` (#1565, #1566).

// Tree-sitter row numbers are source line indices; files with 2^32+ lines
// do not exist in practice, so usize→u32 truncation is safe.
#![allow(clippy::cast_possible_truncation)]

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::types::{Edge, RawCall};

use super::js_extra::is_js_function_value;
use super::names::read_text_owned;

// ── Shadow-name collection (Python) ───────────────────────────────────────────

/// Plain parameter identifiers declared on a Python `parameters` node: positional
/// / keyword params plus `*args` / `**kwargs` and typed or default forms — every
/// name the body binds locally, able to shadow a module-level definition.
fn python_param_names(params: Option<Node<'_>>, source: &[u8], out: &mut HashSet<String>) {
    let Some(params) = params else { return };
    let mut cur = params.walk();
    for child in params.children(&mut cur) {
        match child.kind() {
            "identifier" => {
                out.insert(read_text_owned(child, source));
            }
            "typed_parameter"
            | "default_parameter"
            | "typed_default_parameter"
            | "list_splat_pattern"
            | "dictionary_splat_pattern" => {
                let name_n = child.child_by_field_name("name").or_else(|| {
                    let mut c = child.walk();
                    child.children(&mut c).find(|c| c.kind() == "identifier")
                });
                if let Some(n) = name_n {
                    out.insert(read_text_owned(n, source));
                }
            }
            _ => {}
        }
    }
}

/// Identifiers bound as `pattern` targets under a Python subtree, recursing
/// through `pattern_list` / `tuple_pattern` / `list_pattern` so tuple unpacking
/// (`a, b = ...`, `for a, b in ...`) contributes every bound name.
fn python_collect_assignment_targets(
    node: Option<Node<'_>>,
    source: &[u8],
    out: &mut HashSet<String>,
) {
    let Some(node) = node else { return };
    match node.kind() {
        "identifier" => {
            out.insert(read_text_owned(node, source));
        }
        "pattern_list" | "tuple_pattern" | "list_pattern" => {
            let mut cur = node.walk();
            for c in node.children(&mut cur) {
                python_collect_assignment_targets(Some(c), source, out);
            }
        }
        _ => {}
    }
}

/// Names bound LOCALLY inside a Python function: parameters plus assignment,
/// `for`, `with ... as`, and walrus targets. An argument in this set names a local
/// value, not the module-level function it shares a name with. Nested function /
/// class / lambda subtrees are not descended — their bindings are a different
/// scope. Mirrors `_python_local_bound_names`.
#[must_use]
pub(super) fn python_local_bound_names(func_def: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut bound = HashSet::new();
    python_param_names(
        func_def.child_by_field_name("parameters"),
        source,
        &mut bound,
    );
    if let Some(body) = func_def.child_by_field_name("body") {
        python_walk_bindings(body, source, &mut bound, true);
    }
    bound
}

/// Recursion shared by [`python_local_bound_names`] and [`python_module_bound_names`]:
/// collect assignment / `for` / `with`-as / walrus targets, never descending into a
/// nested function / class / lambda scope.
fn python_walk_bindings(
    node: Node<'_>,
    source: &[u8],
    bound: &mut HashSet<String>,
    include_with: bool,
) {
    let mut cur = node.walk();
    for child in node.children(&mut cur) {
        match child.kind() {
            "function_definition" | "class_definition" | "lambda" => continue,
            "assignment" | "for_statement" | "for_in_clause" => {
                python_collect_assignment_targets(child.child_by_field_name("left"), source, bound);
            }
            "with_statement" if include_with => {
                let mut c1 = child.walk();
                for item in child.children(&mut c1) {
                    if item.kind() == "with_clause" {
                        let mut c2 = item.walk();
                        for wi in item.children(&mut c2) {
                            if wi.kind() == "with_item" {
                                python_collect_assignment_targets(
                                    wi.child_by_field_name("alias"),
                                    source,
                                    bound,
                                );
                            }
                        }
                    }
                }
            }
            "named_expression" => {
                python_collect_assignment_targets(child.child_by_field_name("name"), source, bound);
            }
            _ => {}
        }
        python_walk_bindings(child, source, bound, include_with);
    }
}

/// Names rebound by assignment at MODULE scope (top-level `x = ...`, `for`, walrus).
/// The module-scope analogue of the per-function shadow set: a dispatch-table value
/// whose name is reassigned to data at module level (`handler = build()`) names that
/// value, not a same-named function. Mirrors `_python_module_bound_names`.
#[must_use]
pub(super) fn python_module_bound_names(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut bound = HashSet::new();
    python_walk_bindings(root, source, &mut bound, false);
    bound
}

// ── Shadow-name collection (JS/TS) ────────────────────────────────────────────

/// JS/TS nodes that open a new scope; binding walks stop at them.
const JS_SCOPE_BOUNDARY: &[&str] = &[
    "function_declaration",
    "function_expression",
    "function",
    "arrow_function",
    "method_definition",
    "class_declaration",
    "class",
    "generator_function",
    "generator_function_declaration",
];

/// Collect binding identifier names from a JS/TS pattern (a parameter, or a
/// declarator LHS), recursing through destructuring but never into a default-value
/// side or a type annotation. Mirrors `_js_collect_pattern_idents`.
fn js_collect_pattern_idents(node: Node<'_>, source: &[u8], bound: &mut HashSet<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            bound.insert(read_text_owned(node, source));
        }
        "type_annotation" => {}
        "assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left") {
                js_collect_pattern_idents(left, source, bound);
            }
        }
        "pair_pattern" => {
            if let Some(val) = node.child_by_field_name("value") {
                js_collect_pattern_idents(val, source, bound);
            }
        }
        _ => {
            let mut cur = node.walk();
            for c in node.children(&mut cur) {
                if c.is_named() {
                    js_collect_pattern_idents(c, source, bound);
                }
            }
        }
    }
}

/// Names bound locally inside a JS/TS function: parameters plus `const`/`let`/`var`
/// declarator targets. Nested function / class scopes are not descended. Mirrors
/// `_js_local_bound_names`.
#[must_use]
pub(super) fn js_local_bound_names(func: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut bound = HashSet::new();
    if let Some(params) = func.child_by_field_name("parameters") {
        js_collect_pattern_idents(params, source, &mut bound);
    }
    if let Some(body) = func.child_by_field_name("body") {
        js_walk_declarators(body, source, &mut bound, false);
    }
    bound
}

/// Module-scope names rebound to NON-function data (`const X = {...}`, `let y = 5`).
/// A declarator whose value is itself a function (`const cb = () => {}`) is EXCLUDED:
/// that name IS a callable a dispatch table should resolve to. Mirrors
/// `_js_module_bound_names`.
#[must_use]
pub(super) fn js_module_bound_names(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut bound = HashSet::new();
    js_walk_declarators(root, source, &mut bound, true);
    bound
}

/// Shared declarator walk for JS/TS shadow sets. When `exclude_fn_values` is set
/// (module scope), a declarator whose value is a function is skipped.
fn js_walk_declarators(
    node: Node<'_>,
    source: &[u8],
    bound: &mut HashSet<String>,
    exclude_fn_values: bool,
) {
    let mut cur = node.walk();
    for c in node.children(&mut cur) {
        if JS_SCOPE_BOUNDARY.contains(&c.kind()) {
            continue;
        }
        if c.kind() == "variable_declarator" {
            let is_fn_value = c
                .child_by_field_name("value")
                .is_some_and(|v| is_js_function_value(v.kind()));
            if !(exclude_fn_values && is_fn_value)
                && let Some(name) = c.child_by_field_name("name")
            {
                js_collect_pattern_idents(name, source, bound);
            }
        }
        js_walk_declarators(c, source, bound, exclude_fn_values);
    }
}

// ── Reference-value candidate collection ──────────────────────────────────────

/// Identifier value-nodes of a Python dict/list/set/tuple literal that are
/// function-reference candidates: dict VALUES (never keys), and list/set/tuple
/// elements. Mirrors `_python_dispatch_value_idents`.
#[must_use]
pub(super) fn python_dispatch_value_idents(coll: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut cur = coll.walk();
    if coll.kind() == "dictionary" {
        for pair in coll.children(&mut cur) {
            if pair.kind() == "pair"
                && let Some(val) = pair.child_by_field_name("value")
                && val.kind() == "identifier"
            {
                out.push(val);
            }
        }
    } else {
        for el in coll.children(&mut cur) {
            if el.kind() == "identifier" {
                out.push(el);
            }
        }
    }
    out
}

/// Identifier value-nodes of a JS/TS object/array literal: object property VALUES
/// and shorthand properties (`{ handler }`), and array elements. Keys and inline
/// methods are not references. Mirrors `_js_dispatch_value_idents`.
#[must_use]
pub(super) fn js_dispatch_value_idents(coll: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut cur = coll.walk();
    if coll.kind() == "object" {
        for c in coll.children(&mut cur) {
            match c.kind() {
                "pair" => {
                    if let Some(val) = c.child_by_field_name("value")
                        && val.kind() == "identifier"
                    {
                        out.push(val);
                    }
                }
                "shorthand_property_identifier" => out.push(c),
                _ => {}
            }
        }
    } else {
        for el in coll.children(&mut cur) {
            if el.kind() == "identifier" {
                out.push(el);
            }
        }
    }
    out
}

/// Identifiers on the VALUE side of a Python assignment RHS or a return: a bare
/// name (`cb = handler`, `return handler`) or the elements of a bare unpack
/// (`a, b = f, g`). A collection literal RHS is a dispatch table reached by the
/// normal recursion, so it is not handled here. Mirrors `_python_ref_value_idents`.
#[must_use]
pub(super) fn python_ref_value_idents(value: Option<Node<'_>>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let Some(value) = value else { return out };
    if value.kind() == "identifier" {
        out.push(value);
    } else if value.kind() == "expression_list" {
        let mut cur = value.walk();
        for ch in value.children(&mut cur) {
            if ch.kind() == "identifier" {
                out.push(ch);
            }
        }
    }
    out
}

/// If `call` is a builtin `getattr(obj, "name"[, default])` whose name argument is
/// a PLAIN string literal, return `(name, string_node)`. A dynamic name (variable,
/// f-string, concatenation) yields `None`, as do the 1-arg form and
/// `obj.getattr(...)` (a method, not the builtin). Mirrors `_getattr_ref_name`.
#[must_use]
pub(super) fn getattr_ref_name<'t>(call: Node<'t>, source: &[u8]) -> Option<(String, Node<'t>)> {
    let func = call.child_by_field_name("function")?;
    if func.kind() != "identifier" || read_text_owned(func, source) != "getattr" {
        return None;
    }
    let args = call.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    let positional: Vec<Node<'t>> = args
        .children(&mut cur)
        .filter(|c| c.is_named() && !matches!(c.kind(), "keyword_argument" | "comment"))
        .collect();
    if positional.len() < 2 {
        return None;
    }
    let name_node = positional[1];
    if name_node.kind() != "string" {
        return None;
    }
    let mut c2 = name_node.walk();
    let children: Vec<Node<'t>> = name_node.children(&mut c2).collect();
    if children.iter().any(|ch| ch.kind() == "interpolation") {
        return None; // f-string — dynamic
    }
    let content = children.iter().find(|ch| ch.kind() == "string_content")?;
    Some((read_text_owned(*content, source), name_node))
}

// ── Emit ──────────────────────────────────────────────────────────────────────

/// Shared state for emitting `indirect_call` edges, bundled so the capture sites in
/// the call walk and the module-level scan share one resolve-and-emit path.
pub(super) struct IndirectState<'a> {
    pub str_path: &'a str,
    /// Case-sensitive `label -> nid` map (unlike the call pass's lowercased map):
    /// an indirect ref binds by exact name, preserving case-sensitivity hardening.
    pub label_to_nid_exact: &'a HashMap<String, String>,
    /// `nid -> source_file`, so a same-named local non-callable (reject) is told
    /// apart from an import-surfaced foreign symbol (defer to cross-file).
    pub nid_to_sf: &'a HashMap<String, String>,
    /// Ids of function / method / class definitions in this file.
    pub callable_def_nids: &'a HashSet<String>,
    pub edges: &'a mut Vec<Edge>,
    pub raw_calls: &'a mut Vec<RawCall>,
    /// Direct `calls` pairs already emitted — an existing direct call pre-empts an
    /// indirect edge to the same target.
    pub seen_call_pairs: &'a HashSet<(String, String)>,
    pub seen_indirect_pairs: &'a mut HashSet<(String, String)>,
}

impl IndirectState<'_> {
    /// Resolve a name referenced AS A VALUE to a real callable def and emit one
    /// INFERRED `indirect_call` edge, deferring an unknown / foreign name to the
    /// cross-file resolver. Scope filtering is the caller's job (see [`emit_ref`]);
    /// a `getattr` string names an attribute and is passed straight through here.
    ///
    /// [`emit_ref`]: Self::emit_ref
    pub fn emit_by_name(
        &mut self,
        ident_name: &str,
        loc: Node<'_>,
        scope_nid: &str,
        context: &str,
    ) {
        let line = loc.start_position().row as u32 + 1;
        let ref_nid = self.label_to_nid_exact.get(ident_name);
        // Defer to the cross-file resolver when the name is not in this file, or
        // resolves to an import-surfaced FOREIGN symbol whose callability lives in
        // another file. The cross-file pass applies the global callable-target check.
        let defer = match ref_nid {
            None => true,
            Some(r) => {
                !self.callable_def_nids.contains(r)
                    && self.nid_to_sf.get(r).map_or("", String::as_str) != self.str_path
            }
        };
        if defer {
            self.raw_calls.push(RawCall {
                caller_nid: scope_nid.to_string(),
                callee: ident_name.to_string(),
                is_member_call: false,
                indirect: true,
                context: Some(context.to_string()),
                source_file: self.str_path.to_string(),
                source_location: format!("L{line}"),
                ..Default::default()
            });
            return;
        }
        let Some(ref_nid) = ref_nid else { return };
        // self-ref, or a same-named LOCAL non-callable data node — no edge.
        if ref_nid == scope_nid || !self.callable_def_nids.contains(ref_nid) {
            return;
        }
        let pair = (scope_nid.to_string(), ref_nid.clone());
        if self.seen_call_pairs.contains(&pair) || !self.seen_indirect_pairs.insert(pair) {
            return;
        }
        self.edges.push(Edge {
            external: false,
            source: scope_nid.to_string(),
            target: ref_nid.clone(),
            relation: "indirect_call".to_string(),
            confidence: "INFERRED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: Some(context.to_string()),
            confidence_score: None,
            deferred: false,
            metadata: None,
        });
    }

    /// A function referenced BY NAME (a call argument, or a dispatch-table value) is
    /// an indirect dependency of `scope_nid`. Rejects an identifier shadowed by a
    /// parameter / local binding (and `self` / `cls`) before resolving. Mirrors
    /// `_emit_indirect_ref`.
    pub fn emit_ref(
        &mut self,
        ident: Option<Node<'_>>,
        scope_nid: &str,
        enclosing_locals: &HashSet<String>,
        context: &str,
        source: &[u8],
    ) {
        let Some(ident) = ident else { return };
        if !matches!(ident.kind(), "identifier" | "shorthand_property_identifier") {
            return;
        }
        let name = read_text_owned(ident, source);
        if enclosing_locals.contains(&name) || name == "self" || name == "cls" {
            return;
        }
        self.emit_by_name(&name, ident, scope_nid, context);
    }
}

// ── Module-level dispatch scan ────────────────────────────────────────────────

/// Scan a Python tree for TOP-LEVEL dispatch tables, aliases, and reflective
/// `getattr` (a route / handler registry attributed to the file node), stopping at
/// function / class boundaries so a method's local table is not re-attributed to
/// the file. Mirrors `_scan_module_dispatch`.
pub(super) fn scan_module_dispatch(
    state: &mut IndirectState<'_>,
    node: Node<'_>,
    file_nid: &str,
    module_bound: &HashSet<String>,
    source: &[u8],
) {
    match node.kind() {
        "function_definition" | "class_definition" => return,
        "dictionary" | "list" | "set" | "tuple" => {
            for ident in python_dispatch_value_idents(node) {
                state.emit_ref(Some(ident), file_nid, module_bound, "collection", source);
            }
        }
        "assignment" => {
            for ident in python_ref_value_idents(node.child_by_field_name("right")) {
                state.emit_ref(Some(ident), file_nid, module_bound, "assignment", source);
            }
        }
        "call" => {
            if let Some((name, loc)) = getattr_ref_name(node, source) {
                state.emit_by_name(&name, loc, file_nid, "getattr");
            }
        }
        _ => {}
    }
    let mut cur = node.walk();
    for c in node.children(&mut cur) {
        scan_module_dispatch(state, c, file_nid, module_bound, source);
    }
}

/// Scan a JS/TS tree for module-level dispatch tables and callback registrations
/// (Express routes `app.get("/", handler)`, event wiring, `setTimeout(fn)`),
/// attributed to the file node. Mirrors `_scan_js_module_dispatch`.
pub(super) fn scan_js_module_dispatch(
    state: &mut IndirectState<'_>,
    node: Node<'_>,
    file_nid: &str,
    module_bound: &HashSet<String>,
    source: &[u8],
) {
    if JS_SCOPE_BOUNDARY.contains(&node.kind()) {
        return;
    }
    match node.kind() {
        "object" | "array" => {
            for ident in js_dispatch_value_idents(node) {
                state.emit_ref(Some(ident), file_nid, module_bound, "collection", source);
            }
        }
        "call_expression" | "new_expression" => {
            if let Some(margs) = node.child_by_field_name("arguments") {
                let mut c = margs.walk();
                for marg in margs.children(&mut c) {
                    if marg.kind() == "identifier" {
                        state.emit_ref(Some(marg), file_nid, module_bound, "argument", source);
                    }
                }
            }
        }
        _ => {}
    }
    let mut cur = node.walk();
    for c in node.children(&mut cur) {
        scan_js_module_dispatch(state, c, file_nid, module_bound, source);
    }
}
