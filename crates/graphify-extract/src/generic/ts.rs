//! TS/JS file-wide receiver-type inference for member-call resolution
//! (#1316/#1630).
//!
//! A TS/JS member call (`this.repo.findById()`, `svc.doThing()`) resolved by bare
//! method name mis-binds across same-named methods in the corpus. This builds a
//! file-wide `name -> TypeName` table so each member-call `RawCall` carries a
//! `receiver_type` for `resolve_typescript_member_calls` to bind by the
//! receiver's declared type. Three sources, first-binding-wins with
//! constructor-injection preferred:
//!
//! - constructor parameter-properties (`constructor(private repo: IUserRepo)`) —
//!   `this.repo` is typed `IUserRepo` (#1316);
//! - local `new` bindings (`const x = new Foo()`) -> `x: Foo` (#1630 Pattern A);
//! - type-annotated parameters (`(svc: Svc)`) -> `svc: Svc` (#1630 Pattern B).
//!
//! Only a bare `type_identifier` (a single class/interface name) is recorded — an
//! array, union, generic, qualified, or predefined type is skipped (precision
//! over recall). Mirrors the constructor-injection scan + `_ts_receiver_type_table`.

use std::collections::HashMap;

use tree_sitter::Node;

use super::names::read_text_owned;

/// File-wide `name -> TypeName` table for a TS/JS tree.
pub(super) fn build_ts_type_table(root: Node<'_>, source: &[u8]) -> HashMap<String, String> {
    let mut table = HashMap::new();
    // Constructor-injection first: it wins on a name clash (populated before the
    // `first-binding-wins` new/param scan below).
    collect_ctor_injection(root, source, &mut table);
    collect_new_and_typed_params(root, source, &mut table);
    table
}

/// The single bare `type_identifier` in a `type_annotation` (`: T`), else `None`
/// (an array / union / generic / qualified / predefined type is skipped). Mirrors
/// `_bare_type_ident`.
fn bare_type_ident(type_annotation: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = type_annotation.walk();
    let mut ident: Option<Node<'_>> = None;
    let mut ident_count = 0u32;
    let mut other_named = false;
    for c in type_annotation.children(&mut cur) {
        if c.kind() == "type_identifier" {
            ident = Some(c);
            ident_count += 1;
        } else if c.is_named() {
            other_named = true;
        }
    }
    if ident_count == 1 && !other_named {
        ident.map(|c| read_text_owned(c, source))
    } else {
        None
    }
}

/// Constructor parameter-property types (`private repo: IUserRepo`) -> the field
/// name maps to its declared type. Only a `required_parameter` carrying an
/// `accessibility_modifier` / `readonly` modifier and a single-`type_identifier`
/// annotation contributes.
fn collect_ctor_injection(root: Node<'_>, source: &[u8], table: &mut HashMap<String, String>) {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "method_definition"
            && n.child_by_field_name("name")
                .is_some_and(|nm| read_text_owned(nm, source) == "constructor")
            && let Some(params) = n.child_by_field_name("parameters")
        {
            let mut pcur = params.walk();
            for p in params.children(&mut pcur) {
                if p.kind() != "required_parameter" {
                    continue;
                }
                let mut mcur = p.walk();
                let has_modifier = p
                    .children(&mut mcur)
                    .any(|c| matches!(c.kind(), "accessibility_modifier" | "readonly"));
                if !has_modifier {
                    continue;
                }
                let (Some(name_n), Some(type_n)) = (
                    p.child_by_field_name("pattern"),
                    p.child_by_field_name("type"),
                ) else {
                    continue;
                };
                let pname = read_text_owned(name_n, source);
                let mut tcur = type_n.walk();
                if let Some(tc) = type_n
                    .children(&mut tcur)
                    .find(|c| c.kind() == "type_identifier")
                {
                    let ptype = read_text_owned(tc, source);
                    if !pname.is_empty() && !ptype.is_empty() {
                        table.insert(pname, ptype);
                    }
                }
            }
        }
        let mut cur = n.walk();
        for c in n.children(&mut cur) {
            stack.push(c);
        }
    }
}

/// Local `new` bindings (`const x = new Foo()`) and type-annotated parameters
/// (`(svc: Svc)`), first-binding-wins (constructor-injection entries already in
/// `table` win). Mirrors `_ts_receiver_type_table`.
fn collect_new_and_typed_params(
    root: Node<'_>,
    source: &[u8],
    table: &mut HashMap<String, String>,
) {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "variable_declarator" => {
                if let (Some(name_n), Some(value)) = (
                    n.child_by_field_name("name"),
                    n.child_by_field_name("value"),
                ) && name_n.kind() == "identifier"
                    && value.kind() == "new_expression"
                    && let Some(ctor) = value.child_by_field_name("constructor")
                    && matches!(ctor.kind(), "identifier" | "type_identifier")
                {
                    let name = read_text_owned(name_n, source);
                    let tname = read_text_owned(ctor, source);
                    if !name.is_empty() && !tname.is_empty() {
                        table.entry(name).or_insert(tname);
                    }
                }
            }
            "required_parameter" | "optional_parameter" => {
                if let (Some(pat), Some(ann)) = (
                    n.child_by_field_name("pattern"),
                    n.child_by_field_name("type"),
                ) && pat.kind() == "identifier"
                    && let Some(tname) = bare_type_ident(ann, source)
                {
                    let name = read_text_owned(pat, source);
                    if !name.is_empty() {
                        table.entry(name).or_insert(tname);
                    }
                }
            }
            _ => {}
        }
        let mut cur = n.walk();
        for c in n.children(&mut cur) {
            stack.push(c);
        }
    }
}
