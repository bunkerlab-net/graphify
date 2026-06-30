//! Java type-reference and annotation collectors.

use std::collections::HashSet;

use tree_sitter::Node;

use super::{RefRole, role_of};
use crate::generic::names::read_text_owned;

/// Declaration kinds that can introduce Java type parameters (`<T>`).
const JAVA_TYPE_PARAMETER_SCOPE_DECLARATIONS: [&str; 5] = [
    "class_declaration",
    "interface_declaration",
    "record_declaration",
    "method_declaration",
    "constructor_declaration",
];

/// Type-parameter names visible from `node` — the `<T>` / `<U>` declared on any
/// enclosing class/interface/record/method/constructor. A reference to one of
/// these names is a type variable, not a real type, so it must emit neither a
/// `references` edge nor a sourceless stub node (#1518).
fn java_type_parameters_in_scope(node: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut scope = Some(node);
    while let Some(s) = scope {
        if JAVA_TYPE_PARAMETER_SCOPE_DECLARATIONS.contains(&s.kind())
            && let Some(params) = s.child_by_field_name("type_parameters")
        {
            let mut cur = params.walk();
            if cur.goto_first_child() {
                loop {
                    let param = cur.node();
                    if param.kind() == "type_parameter" {
                        let mut pcur = param.walk();
                        if pcur.goto_first_child() {
                            loop {
                                if pcur.node().kind() == "type_identifier" {
                                    names.insert(read_text_owned(pcur.node(), source));
                                    break;
                                }
                                if !pcur.goto_next_sibling() {
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
        scope = s.parent();
    }
    names
}

/// Walk a Java type expression, appending `(name, role)` references. Type-
/// parameter names in scope are skipped (#1518); the scope is computed once
/// from `node` and threaded through the recursion.
pub(crate) fn java_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    let skip = java_type_parameters_in_scope(node, source);
    java_collect_type_refs_inner(node, source, generic, out, &skip);
}

#[allow(clippy::too_many_lines)] // single recursive dispatch over tree-sitter Java type kinds
fn java_collect_type_refs_inner(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
    skip: &HashSet<String>,
) {
    let t = node.kind();
    if matches!(
        t,
        "integral_type" | "floating_point_type" | "boolean_type" | "void_type"
    ) {
        return;
    }
    if t == "type_identifier" {
        let name = read_text_owned(node, source);
        if !name.is_empty() && !skip.contains(&name) {
            let role = role_of(generic);
            out.push((name, role));
        }
        return;
    }
    if t == "scoped_type_identifier" {
        let text = read_text_owned(node, source);
        let tail = text.rsplit('.').next().unwrap_or(&text);
        if !tail.is_empty() {
            let role = role_of(generic);
            out.push((tail.to_string(), role));
        }
        return;
    }
    if t == "generic_type" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if matches!(child.kind(), "type_identifier" | "scoped_type_identifier") {
                    let text = read_text_owned(child, source);
                    let tail = text.rsplit('.').next().unwrap_or(&text);
                    // A bare `type_identifier` that names a type parameter is
                    // skipped; a `scoped_type_identifier` (e.g. `a.b.C`) is never a
                    // type parameter and is always kept (#1518).
                    if !tail.is_empty()
                        && (child.kind() == "scoped_type_identifier" || !skip.contains(tail))
                    {
                        let role = role_of(generic);
                        out.push((tail.to_string(), role));
                    }
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "type_arguments" {
                    let mut acur = child.walk();
                    if acur.goto_first_child() {
                        loop {
                            if acur.node().is_named() {
                                java_collect_type_refs_inner(acur.node(), source, true, out, skip);
                            }
                            if !acur.goto_next_sibling() {
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
        return;
    }
    if t == "array_type" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    java_collect_type_refs_inner(cur.node(), source, generic, out, skip);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }
    if node.is_named() {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    java_collect_type_refs_inner(cur.node(), source, generic, out, skip);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Find the `modifiers` child of a Java method declaration, if any.
fn find_modifiers(method_node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = method_node.walk();
    if !cur.goto_first_child() {
        return None;
    }
    loop {
        if cur.node().kind() == "modifiers" {
            return Some(cur.node());
        }
        if !cur.goto_next_sibling() {
            return None;
        }
    }
}

/// Collect annotation names from a Java declaration's `modifiers` child
/// (a class, interface, record, or method) (#1487).
///
/// `@Override @Deprecated public void foo()` yields `["Override", "Deprecated"]`.
#[must_use]
pub(crate) fn java_annotation_names(declaration_node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let Some(modifiers) = find_modifiers(declaration_node) else {
        return names;
    };
    let mut acur = modifiers.walk();
    if !acur.goto_first_child() {
        return names;
    }
    loop {
        let anno = acur.node();
        if matches!(anno.kind(), "marker_annotation" | "annotation") {
            let name_node = anno.child_by_field_name("name").or_else(|| {
                let mut sc = anno.walk();
                if sc.goto_first_child() {
                    loop {
                        if matches!(
                            sc.node().kind(),
                            "identifier" | "scoped_identifier" | "type_identifier"
                        ) {
                            return Some(sc.node());
                        }
                        if !sc.goto_next_sibling() {
                            break;
                        }
                    }
                }
                None
            });
            if let Some(nn) = name_node {
                let text = read_text_owned(nn, source);
                let tail = text.rsplit('.').next().unwrap_or(&text);
                if !tail.is_empty() {
                    names.push(tail.to_string());
                }
            }
        }
        if !acur.goto_next_sibling() {
            break;
        }
    }
    names
}
