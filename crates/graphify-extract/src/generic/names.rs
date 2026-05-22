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

#[must_use]
pub fn get_cpp_func_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
            return Some(read_text_owned(node, source));
        }
        "qualified_identifier" => {
            if let Some(name) = node.child_by_field_name("name") {
                return Some(read_text_owned(name, source));
            }
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

/// Extract a simple type name from an arbitrary C# type node.
///
/// Handles `identifier`, `predefined_type`, `qualified_name`, `generic_name`,
/// and falls back to recursing into child nodes. Returns `None` when no
/// recognisable name can be extracted (e.g. array/pointer type modifiers).
pub(super) fn read_csharp_type_name(node: Option<Node<'_>>, source: &[u8]) -> Option<String> {
    let node = node?;
    match node.kind() {
        "identifier" | "predefined_type" => Some(read_text_owned(node, source)),
        "qualified_name" => {
            let text = read_text_owned(node, source);
            Some(text.split('.').next_back().unwrap_or("").to_string())
        }
        "generic_name" => node
            .child_by_field_name("name")
            .map(|n| read_text_owned(n, source)),
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.is_named()
                        && let Some(n) = read_csharp_type_name(Some(child), source)
                        && !n.is_empty()
                    {
                        return Some(n);
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
