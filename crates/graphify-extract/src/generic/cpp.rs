//! C++ per-function-body receiver-type inference for member-call resolution (#1547).
//!
//! A C++ member call (`f.bar()` / `f->bar()`) resolved by bare method name
//! silently mis-binds across same-named methods in the corpus. This collects a
//! per-function-body, first-binding-wins `var -> ClassName` table from local
//! variable declarations in that body (`Foo f;`, `Foo* f;`, `Foo *f = ...;`,
//! `Foo f = Foo();`) so each member-call `RawCall` carries a `receiver_type` for
//! `resolve_cpp_member_calls` to bind by the receiver's declared type.
//!
//! Only a class-like (`type_identifier` / `qualified_identifier`) type with a
//! single named declarator is recorded — precision over recall: a built-in type
//! (`int x`), an ambiguous multi-declarator line, or an un-nameable declarator
//! contributes nothing rather than a guess. A qualified type `ns::Foo` records
//! its simple tail `Foo`. Mirrors graphify-py `_cpp_local_var_types` /
//! `_cpp_declarator_name`.

use std::collections::HashMap;

use tree_sitter::Node;

use super::names::read_text_owned;

/// Bare variable name from a C++ declaration declarator, unwrapping
/// pointer/reference/init wrappers (`*f`, `&r`, `f = Foo()`). `None` for anything
/// that isn't a plain named local (arrays, function pointers, structured
/// bindings) so the type table never records a guessed receiver. Mirrors
/// `_cpp_declarator_name`.
fn cpp_declarator_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => Some(read_text_owned(node, source)),
        "pointer_declarator" | "reference_declarator" | "init_declarator" => {
            let inner = node.child_by_field_name("declarator").or_else(|| {
                let mut cur = node.walk();
                node.children(&mut cur).find(|c| {
                    matches!(
                        c.kind(),
                        "identifier" | "pointer_declarator" | "reference_declarator"
                    )
                })
            });
            inner.and_then(|c| cpp_declarator_name(c, source))
        }
        _ => None,
    }
}

/// Collect `var -> ClassName` from local variable declarations in a C++ function
/// body into `table` (first-binding-wins: a later `Foo f;` never clobbers an
/// earlier binding). Skips nested functions / lambdas so their locals don't
/// pollute this body's scope. Ports `_cpp_local_var_types`.
pub(super) fn collect_cpp_local_var_types(
    body: Node<'_>,
    source: &[u8],
    table: &mut HashMap<String, String>,
) {
    let mut stack = vec![body];
    while let Some(n) = stack.pop() {
        // Don't descend into a nested function / lambda: its locals are scoped
        // away and would pollute this body's table.
        if matches!(n.kind(), "function_definition" | "lambda_expression") && n.id() != body.id() {
            continue;
        }
        if n.kind() == "declaration"
            && let Some(type_node) = n.child_by_field_name("type")
            && matches!(type_node.kind(), "type_identifier" | "qualified_identifier")
        {
            let type_name = read_text_owned(type_node, source)
                .rsplit("::")
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            let mut cur = n.walk();
            let declarators: Vec<Node<'_>> = n
                .children(&mut cur)
                .filter(|c| {
                    matches!(
                        c.kind(),
                        "identifier"
                            | "pointer_declarator"
                            | "reference_declarator"
                            | "init_declarator"
                    )
                })
                .collect();
            // A single declarator only: `Foo a, b;` is ambiguous to attribute to
            // one receiver name cleanly, so skip multi-declarator lines.
            if !type_name.is_empty()
                && type_name.chars().next().is_some_and(char::is_uppercase)
                && declarators.len() == 1
                && let Some(var) = cpp_declarator_name(declarators[0], source)
                && !table.contains_key(&var)
            {
                table.insert(var, type_name);
            }
        }
        // Visit children in source order so a shadowed local's FIRST declaration
        // wins, honouring the first-binding-wins contract above. Divergence from
        // graphify-py `_cpp_local_var_types`: its forward-push LIFO stack reaches
        // later siblings first (an accidental order no test pins); walking the
        // cursor backwards restores source order.
        let mut cur = n.walk();
        if cur.goto_last_child() {
            loop {
                stack.push(cur.node());
                if !cur.goto_previous_sibling() {
                    break;
                }
            }
        }
    }
}
