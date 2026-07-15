//! Java local receiver-type inference for member-call resolution (#1696).
//!
//! A Java member call (`gw.charge()`) can't be resolved by bare method name — the
//! name collides across the corpus. This builds a method-scoped `receiver -> type`
//! table (current-class fields + method parameters + explicit locals, plus
//! `this.field` entries) so each member-call `RawCall` carries a `receiver_type`
//! for `resolve_java_member_calls` to bind by the receiver's declared type.
//!
//! The table is deliberately conservative: a name bound to two different types (a
//! local shadowing a field with a different type, or two locals) is dropped as
//! ambiguous, because raw calls retain no lexical scope. Mirrors graphify-py
//! `_java_method_receiver_types` / `_java_receiver_type_name`.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::names::read_text_owned;
use super::references::{is_java_builtin, java_type_parameters_in_scope};

/// The concrete declared type usable for Java receiver resolution, or `None` for
/// a primitive, a type parameter, or an unsupported type shape. Mirrors
/// graphify-py `_java_receiver_type_name`.
fn java_receiver_type_name(type_node: Option<Node<'_>>, source: &[u8]) -> Option<String> {
    let type_node = type_node?;
    let name = match type_node.kind() {
        "type_identifier" => read_text_owned(type_node, source),
        "scoped_type_identifier" => {
            let full = read_text_owned(type_node, source);
            full.rsplit('.').next().unwrap_or(&full).to_string()
        }
        "generic_type" => {
            // Resolve against the base type_identifier / scoped_type_identifier.
            let mut base = None;
            let mut cur = type_node.walk();
            if cur.goto_first_child() {
                loop {
                    let c = cur.node();
                    if matches!(c.kind(), "type_identifier" | "scoped_type_identifier") {
                        base = Some(c);
                        break;
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            return java_receiver_type_name(base, source);
        }
        _ => return None,
    };
    if name.is_empty()
        || is_java_builtin(&name)
        || java_type_parameters_in_scope(type_node, source).contains(&name)
    {
        return None;
    }
    Some(name)
}

/// Variable names declared by a `field_declaration` / `local_variable_declaration`
/// (its `variable_declarator` children). Mirrors `_java_declarator_names`.
fn java_declarator_names(declaration_node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = declaration_node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "variable_declarator"
                && let Some(name_node) = child.child_by_field_name("name")
            {
                let name = read_text_owned(name_node, source);
                if !name.is_empty() {
                    names.push(name);
                }
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    names
}

/// `(name, declared type)` bindings of a lambda's parameters. Untyped params
/// (`x -> …`, `(a, b) -> …`) carry `None`. Mirrors `_java_lambda_parameters`.
fn java_lambda_parameters(lambda_node: Node<'_>, source: &[u8]) -> Vec<(String, Option<String>)> {
    let Some(parameters) = lambda_node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match parameters.kind() {
        "identifier" => out.push((read_text_owned(parameters, source), None)),
        "inferred_parameters" => {
            let mut cur = parameters.walk();
            if cur.goto_first_child() {
                loop {
                    let c = cur.node();
                    if c.kind() == "identifier" {
                        out.push((read_text_owned(c, source), None));
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        _ => {
            let mut cur = parameters.walk();
            if cur.goto_first_child() {
                loop {
                    let param = cur.node();
                    if matches!(param.kind(), "formal_parameter" | "spread_parameter")
                        && let Some(name_node) = param.child_by_field_name("name")
                    {
                        out.push((
                            read_text_owned(name_node, source),
                            java_receiver_type_name(param.child_by_field_name("type"), source),
                        ));
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
    out
}

/// Bind `name -> type_name` unless the name is empty/untyped or already bound to a
/// *different* type (then drop it as ambiguous). Mirrors the inner `bind`.
fn bind(
    name: &str,
    type_name: Option<&str>,
    method_types: &mut HashMap<String, String>,
    ambiguous: &mut HashSet<String>,
) {
    if name.is_empty() || ambiguous.contains(name) {
        return;
    }
    let Some(tn) = type_name else {
        return;
    };
    match method_types.get(name) {
        Some(prev) if prev != tn => {
            method_types.remove(name);
            ambiguous.insert(name.to_string());
        }
        _ => {
            method_types.insert(name.to_string(), tn.to_string());
        }
    }
}

/// `true` when a class field of the same name has a *different* declared type,
/// so a local/lambda binding must be dropped as ambiguous rather than shadow it.
/// Mirrors Python `field_types.get(name) not in (None, type_name)`.
fn field_conflict(
    field_types: &HashMap<String, String>,
    name: &str,
    type_name: Option<&str>,
) -> bool {
    field_types
        .get(name)
        .is_some_and(|ft| Some(ft.as_str()) != type_name)
}

/// Push every child of `node` onto `stack` (all children, named or not).
fn push_children<'t>(node: Node<'t>, stack: &mut Vec<Node<'t>>) {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            stack.push(cur.node());
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Build the receiver-type table visible to one Java method: current-class fields
/// (base scope) overlaid by parameters and explicit locals, minus any name bound
/// ambiguously, plus a `this.field` entry per field. Mirrors graphify-py
/// `_java_method_receiver_types`.
fn java_method_receiver_types(
    method_node: Node<'_>,
    source: &[u8],
    field_types: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut method_types: HashMap<String, String> = HashMap::new();
    let mut ambiguous: HashSet<String> = HashSet::new();

    // Parameters shadow fields for the whole method.
    if let Some(params) = method_node.child_by_field_name("parameters") {
        let mut cur = params.walk();
        if cur.goto_first_child() {
            loop {
                let param = cur.node();
                if matches!(param.kind(), "formal_parameter" | "spread_parameter") {
                    let type_name =
                        java_receiver_type_name(param.child_by_field_name("type"), source);
                    if let Some(name_node) = param.child_by_field_name("name") {
                        bind(
                            &read_text_owned(name_node, source),
                            type_name.as_deref(),
                            &mut method_types,
                            &mut ambiguous,
                        );
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    // Body locals + lambda params, skipping nested type declarations (which own
    // their own scope). Raw calls are method-scoped, so a lambda-local binding
    // cannot be told apart from an enclosing one of the same name — drop it.
    if let Some(body) = method_node.child_by_field_name("body") {
        let mut stack: Vec<Node<'_>> = Vec::new();
        push_children(body, &mut stack);
        while let Some(node) = stack.pop() {
            match node.kind() {
                "class_declaration"
                | "class_body"
                | "interface_declaration"
                | "record_declaration"
                | "enum_declaration"
                | "annotation_type_declaration" => continue,
                "lambda_expression" => {
                    for (name, type_name) in java_lambda_parameters(node, source) {
                        if type_name.is_none()
                            || field_conflict(field_types, &name, type_name.as_deref())
                        {
                            method_types.remove(&name);
                            ambiguous.insert(name);
                        } else {
                            bind(
                                &name,
                                type_name.as_deref(),
                                &mut method_types,
                                &mut ambiguous,
                            );
                        }
                    }
                }
                "local_variable_declaration" => {
                    let type_name =
                        java_receiver_type_name(node.child_by_field_name("type"), source);
                    for name in java_declarator_names(node, source) {
                        if field_conflict(field_types, &name, type_name.as_deref()) {
                            method_types.remove(&name);
                            ambiguous.insert(name);
                        } else {
                            bind(
                                &name,
                                type_name.as_deref(),
                                &mut method_types,
                                &mut ambiguous,
                            );
                        }
                    }
                }
                _ => {}
            }
            push_children(node, &mut stack);
        }
    }

    let mut table: HashMap<String, String> = field_types.clone();
    table.extend(method_types);
    for name in &ambiguous {
        table.remove(name);
    }
    for (name, tn) in field_types {
        table.insert(format!("this.{name}"), tn.clone());
    }
    table
}

/// Field name -> declared type for the class enclosing `method_node` (its direct
/// `field_declaration` members). Base scope for [`java_method_receiver_types`].
fn enclosing_class_field_types(method_node: Node<'_>, source: &[u8]) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let Some(class_body) = method_node.parent() else {
        return fields;
    };
    let mut cur = class_body.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "field_declaration"
                && let Some(type_name) =
                    java_receiver_type_name(child.child_by_field_name("type"), source)
            {
                for name in java_declarator_names(child, source) {
                    fields.insert(name, type_name.clone());
                }
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    fields
}

/// The Java receiver-type table for a single method/constructor body, or empty
/// for anything else. Keyed per-body (not per-method NID) so overloaded methods
/// — which collapse to one NID but keep distinct bodies with different local
/// types — each resolve against their own scope. Consumed in the call walk to
/// attach a `receiver_type` to member-call `RawCall`s (#1696).
#[must_use]
pub(super) fn build_java_receiver_types_for_body(
    body: Node<'_>,
    source: &[u8],
) -> HashMap<String, String> {
    let Some(method_node) = body.parent() else {
        return HashMap::new();
    };
    if !matches!(
        method_node.kind(),
        "method_declaration" | "constructor_declaration"
    ) {
        return HashMap::new();
    }
    let field_types = enclosing_class_field_types(method_node, source);
    java_method_receiver_types(method_node, source, &field_types)
}
