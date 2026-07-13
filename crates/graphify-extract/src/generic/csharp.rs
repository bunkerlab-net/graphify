//! C# file-wide receiver-type inference for member-call resolution (#1609).
//!
//! A C# member call (`recv.Method()`) resolved by bare method name silently
//! mis-bound `_server.Save()` to an unrelated `Cache.Save()`. This builds a
//! file-wide, first-binding-wins `name -> Type` table from fields, properties,
//! parameters, and locals (including `var v = new T()`) so each member-call
//! `RawCall` carries a `receiver_type` for `resolve_csharp_member_calls` to bind
//! by the receiver's declared type.
//!
//! Only a resolvable, Pascal-cased type name is recorded; primitives and a bare
//! `var` without a `new T()` initializer are skipped (precision over recall — an
//! untypable receiver is left for the resolver to drop rather than guess).
//! Mirrors graphify-py `_csharp_member_type_table`.

use std::collections::HashMap;

use tree_sitter::Node;

use super::names::{read_csharp_type_name, read_text_owned};
use super::walk::first_child_kind;

/// A resolvable, Pascal-cased C# type name for `type_node`, else `None` (skips
/// primitives / lower-cased names that own no resolvable definition here).
fn csharp_typed(type_node: Option<Node<'_>>, source: &[u8]) -> Option<String> {
    let name = read_csharp_type_name(type_node, source)?.name;
    if name.chars().next().is_some_and(char::is_uppercase) {
        Some(name)
    } else {
        None
    }
}

/// `(name, declarator node)` for each `variable_declarator` under a
/// `variable_declaration`. Mirrors `_decl_names`.
fn decl_names<'t>(var_decl: Node<'t>, source: &[u8]) -> Vec<(String, Node<'t>)> {
    let mut out = Vec::new();
    let mut cur = var_decl.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "variable_declarator"
                && let Some(nm) = c
                    .child_by_field_name("name")
                    .or_else(|| first_child_kind(c, "identifier"))
            {
                out.push((read_text_owned(nm, source), c));
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

/// Type of `var v = new T()` recovered from the declarator's
/// `object_creation_expression`. Mirrors `_new_type`.
fn new_type(declarator: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = declarator.walk();
    if cur.goto_first_child() {
        loop {
            let g = cur.node();
            if g.kind() == "object_creation_expression" {
                return csharp_typed(g.child_by_field_name("type"), source);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Build the file-wide `name -> Type` table (fields, properties, parameters,
/// locals), first-binding-wins. Mirrors graphify-py `_csharp_member_type_table`.
#[must_use]
pub(super) fn build_csharp_type_table(root: Node<'_>, source: &[u8]) -> HashMap<String, String> {
    let mut table: HashMap<String, String> = HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "field_declaration" | "local_declaration_statement" => {
                if let Some(vd) = first_child_kind(n, "variable_declaration") {
                    let declared = csharp_typed(vd.child_by_field_name("type"), source);
                    for (name, decl) in decl_names(vd, source) {
                        if name.is_empty() {
                            continue;
                        }
                        if let Some(resolved) = declared.clone().or_else(|| new_type(decl, source))
                        {
                            table.entry(name).or_insert(resolved);
                        }
                    }
                }
            }
            "property_declaration" | "parameter" => {
                if let Some(nm) = n.child_by_field_name("name")
                    && let Some(resolved) = csharp_typed(n.child_by_field_name("type"), source)
                {
                    table.entry(read_text_owned(nm, source)).or_insert(resolved);
                }
            }
            _ => {}
        }
        let mut cur = n.walk();
        if cur.goto_first_child() {
            loop {
                stack.push(cur.node());
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    table
}
