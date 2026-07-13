//! Name and text resolution helpers for C, C++, and C#.
//!
//! Contains `get_c_func_name`, `get_cpp_func_name`, `read_csharp_type_name`,
//! plus the low-level `read_text` / `read_text_owned` byte-slice helpers used
//! throughout the generic extractor.

use tree_sitter::Node;

// ── Text helpers ──────────────────────────────────────────────────────────────

/// Return the source text covered by `node` as a `&str`, or `""` on bad UTF-8.
///
/// The lifetime `'a` is tied to `source`, not to `node`, so the returned
/// slice is valid for the duration of the parse session.
pub(super) fn read_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Return the source text covered by `node` as an owned `String`.
///
/// Uses lossy UTF-8 conversion so malformed bytes produce replacement
/// characters rather than panicking, matching Python's behaviour.
pub(super) fn read_text_owned(node: Node<'_>, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).into_owned()
}

// ── C function-name resolver ──────────────────────────────────────────────────

/// Recursively extract the function name from a C declarator subtree.
///
/// Descends through nested `declarator` fields and sibling `identifier` nodes
/// to handle pointer and array declarators that wrap the actual name.
#[must_use]
pub fn get_c_func_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(read_text_owned(node, source));
    }
    if let Some(decl) = node.child_by_field_name("declarator") {
        return get_c_func_name(decl, source);
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "identifier" {
                return Some(read_text_owned(child, source));
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

// ── C++ function-name resolver ────────────────────────────────────────────────

/// Recursively extract the function name from a C++ declarator subtree.
///
/// Handles qualified identifiers, destructors, operator overloads, and nested
/// `declarator` chains in addition to the simpler C cases.
#[must_use]
pub fn get_cpp_func_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        // `qualified_identifier`: an out-of-class DEFINITION (`void Foo::bar() {}`)
        // carries a qualified declarator. Returning the full text retains the
        // `Foo::` qualifier so `make_id(stem, "Foo::bar")` normalizes to the same
        // id as the in-class member `make_id(class_id, "bar")` — the decl in Foo.h
        // and the def in Foo.cpp resolve to ONE method node, not two (#1547). It
        // also handles nested scopes (`A::B::bar`). Free functions never carry a
        // qualified_identifier here, so their bare-name ids are unchanged.
        "identifier"
        | "field_identifier"
        | "destructor_name"
        | "operator_name"
        | "qualified_identifier" => {
            return Some(read_text_owned(node, source));
        }
        _ => {}
    }
    if let Some(decl) = node.child_by_field_name("declarator") {
        return get_cpp_func_name(decl, source);
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "identifier" {
                return Some(read_text_owned(child, source));
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

// ── C# type-name resolver ─────────────────────────────────────────────────────

/// A C# type reference decomposed into its simple `name`, whether the source
/// wrote it `qualified` (`Namespace.Type`), and the `qualifier` prefix. Mirrors
/// graphify-py `_read_csharp_type_name`'s `(name, qualified, qualifier)` tuple.
pub(crate) struct CsharpTypeName {
    pub name: String,
    pub qualified: bool,
    pub qualifier: String,
}

/// Decompose an arbitrary C# type node into [`CsharpTypeName`].
///
/// Handles `identifier`, `predefined_type`, `qualified_name`, `generic_name`,
/// and falls back to recursing into child nodes. Returns `None` when no
/// recognisable name can be extracted (e.g. array/pointer type modifiers).
#[allow(clippy::similar_names)] // `qualified` (bool) vs `qualifier` (prefix) mirror graphify-py
pub(crate) fn read_csharp_type_name(
    node: Option<Node<'_>>,
    source: &[u8],
) -> Option<CsharpTypeName> {
    let node = node?;
    match node.kind() {
        "identifier" | "predefined_type" => Some(CsharpTypeName {
            name: read_text_owned(node, source),
            qualified: false,
            qualifier: String::new(),
        }),
        "qualified_name" => {
            let text = read_text_owned(node, source);
            let (qualifier, tail) = text
                .rsplit_once('.')
                .map_or((String::new(), text.clone()), |(p, t)| {
                    (p.to_string(), t.to_string())
                });
            let name = tail.split('<').next().unwrap_or(&tail).to_string();
            Some(CsharpTypeName {
                name,
                qualified: true,
                qualifier,
            })
        }
        "generic_name" => {
            let name_node = node.child_by_field_name("name")?;
            let qualified = name_node.kind() == "qualified_name";
            let full = read_text_owned(name_node, source);
            let (qualifier, tail) = full
                .rsplit_once('.')
                .map_or((String::new(), full.clone()), |(p, t)| {
                    (p.to_string(), t.to_string())
                });
            Some(CsharpTypeName {
                name: tail,
                qualified,
                qualifier: if qualified { qualifier } else { String::new() },
            })
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.is_named()
                        && let Some(r) = read_csharp_type_name(Some(child), source)
                        && !r.name.is_empty()
                    {
                        return Some(r);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        }
    }
}
