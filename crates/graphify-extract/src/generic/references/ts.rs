//! TypeScript / JavaScript type-reference and heritage collectors.

use tree_sitter::Node;

use super::{RefRole, role_of};
use crate::generic::names::read_text_owned;

/// TS/JS primitive type names that are emitted by tree-sitter as `identifier`
/// or `type_identifier` but do not represent user-defined types. We skip them
/// when collecting reference names to avoid noise like `string` / `number`.
///
/// Mirrors `_JS_PRIMITIVE_TYPES` in `extract.py`.
const JS_PRIMITIVE_TYPES: &[&str] = &[
    "string",
    "number",
    "boolean",
    "any",
    "unknown",
    "void",
    "never",
    "object",
    "null",
    "undefined",
    "bigint",
    "symbol",
    "this",
];

fn is_js_primitive(name: &str) -> bool {
    JS_PRIMITIVE_TYPES.contains(&name)
}

/// Walk a TypeScript type annotation tree and append `(name, role)` tuples.
///
/// Mirrors Python `_ts_collect_type_refs`.
#[allow(clippy::too_many_lines)] // single recursive dispatch over tree-sitter TypeScript type kinds
pub(crate) fn ts_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    let t = node.kind();
    if t == "type_annotation" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    ts_collect_type_refs(cur.node(), source, generic, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }
    if matches!(t, "type_identifier" | "identifier") {
        let name = read_text_owned(node, source);
        if !name.is_empty() && !is_js_primitive(&name) {
            let role = role_of(generic);
            out.push((name, role));
        }
        return;
    }
    if t == "nested_type_identifier" {
        let text = read_text_owned(node, source);
        let tail = text.rsplit('.').next().unwrap_or(&text);
        if !tail.is_empty() && !is_js_primitive(tail) {
            let role = role_of(generic);
            out.push((tail.to_string(), role));
        }
        return;
    }
    if t == "generic_type" {
        let name_node = node.child_by_field_name("name");
        if let Some(nn) = name_node {
            let text = read_text_owned(nn, source);
            let tail = text.rsplit('.').next().unwrap_or(&text);
            if !tail.is_empty() && !is_js_primitive(tail) {
                let role = role_of(generic);
                out.push((tail.to_string(), role));
            }
        } else {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if matches!(
                        cur.node().kind(),
                        "type_identifier" | "nested_type_identifier"
                    ) {
                        let text = read_text_owned(cur.node(), source);
                        let tail = text.rsplit('.').next().unwrap_or(&text);
                        if !tail.is_empty() && !is_js_primitive(tail) {
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
        }
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "type_arguments" {
                    let mut acur = cur.node().walk();
                    if acur.goto_first_child() {
                        loop {
                            if acur.node().is_named() {
                                ts_collect_type_refs(acur.node(), source, true, out);
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
    if node.is_named() {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    ts_collect_type_refs(cur.node(), source, generic, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Return the type-identifier names extracted from an `extends_clause` or
/// `implements_clause`. Both clauses can list multiple types (e.g.
/// `implements A, B<C>`); each name is returned as the tail of any
/// qualified path (`Foo.Bar` → `"Bar"`).
///
/// Mirrors Python `_ts_heritage_clause_entries`.
#[must_use]
pub(crate) fn ts_heritage_clause_entries(clause: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = clause.walk();
    if !cur.goto_first_child() {
        return out;
    }
    loop {
        let child = cur.node();
        if child.is_named() {
            match child.kind() {
                "identifier" | "type_identifier" => {
                    let name = read_text_owned(child, source);
                    if !name.is_empty() {
                        out.push(name);
                    }
                }
                "generic_type" => {
                    let name_node = child.child_by_field_name("name").or_else(|| {
                        let mut sc = child.walk();
                        if sc.goto_first_child() {
                            loop {
                                if matches!(
                                    sc.node().kind(),
                                    "type_identifier" | "nested_type_identifier" | "identifier"
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
                            out.push(tail.to_string());
                        }
                    }
                }
                "nested_type_identifier" => {
                    let text = read_text_owned(child, source);
                    let tail = text.rsplit('.').next().unwrap_or(&text);
                    if !tail.is_empty() {
                        out.push(tail.to_string());
                    }
                }
                _ => {}
            }
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    out
}
