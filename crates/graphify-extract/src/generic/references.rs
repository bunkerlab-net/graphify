//! Per-language type-reference emitters for function/method declarations.
//!
//! These helpers walk parameter lists, return types, and annotations on
//! function nodes to emit `references` edges with a `context` set to
//! `parameter_type`, `return_type`, `generic_arg`, or `attribute`.
//!
//! Mirrors the `_python_*` / `_csharp_*` / `_java_*` helpers added to
//! `graphify-py/graphify/extract.py` in ab4e542.

use tree_sitter::Node;

use super::names::read_text_owned;

/// Role of a collected type reference. `Direct` = used as the type itself
/// (e.g. `def f(x: Foo)`), `Generic` = used as a type argument to a generic
/// (e.g. `def f(x: list[Foo])`).
#[derive(Clone, Copy)]
pub(super) enum RefRole {
    Direct,
    Generic,
}

impl RefRole {
    /// Map a role into the canonical `context` string used on the emitted edge.
    /// `Direct` becomes the supplied `direct_ctx` (e.g. `"parameter_type"` or
    /// `"return_type"`); `Generic` always becomes `"generic_arg"`.
    pub(super) fn into_context(self, direct_ctx: &'static str) -> &'static str {
        match self {
            Self::Direct => direct_ctx,
            Self::Generic => "generic_arg",
        }
    }
}

// ── Python ────────────────────────────────────────────────────────────────────

/// Python `typing` containers that are not themselves user-defined types and
/// must therefore be skipped when collecting reference names — but their
/// nested arguments still count as `generic_arg`.
///
/// Mirrors `_PYTHON_TYPE_CONTAINERS` in `extract.py`.
const PYTHON_TYPE_CONTAINERS: &[&str] = &[
    "list",
    "dict",
    "set",
    "tuple",
    "frozenset",
    "type",
    "List",
    "Dict",
    "Set",
    "Tuple",
    "FrozenSet",
    "Type",
    "Optional",
    "Union",
    "Sequence",
    "Iterable",
    "Mapping",
    "MutableMapping",
    "Iterator",
    "Callable",
    "Awaitable",
    "AsyncIterable",
    "AsyncIterator",
    "Coroutine",
    "Generator",
    "AsyncGenerator",
    "ContextManager",
    "AsyncContextManager",
    "Annotated",
    "ClassVar",
    "Final",
    "Literal",
    "Concatenate",
    "ParamSpec",
    "TypeVar",
    "None",
    "Ellipsis",
];

fn is_python_container(name: &str) -> bool {
    PYTHON_TYPE_CONTAINERS.contains(&name)
}

/// Walk a Python type annotation tree and append `(name, role)` pairs.
///
/// `generic = true` means we entered the function from a `subscript` value or
/// `type_arguments` child, so every emitted name takes the `Generic` role.
#[allow(clippy::too_many_lines)] // single recursive dispatch over tree-sitter Python type kinds; splitting would fragment the per-kind branches
pub(super) fn python_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    let t = node.kind();
    if t == "type" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.is_named() {
                    python_collect_type_refs(child, source, generic, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }
    if t == "identifier" {
        let name = read_text_owned(node, source);
        if !name.is_empty() && !is_python_container(&name) {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((name, role));
        }
        return;
    }
    if t == "attribute" {
        let text = read_text_owned(node, source);
        let tail = text.rsplit('.').next().unwrap_or(&text);
        if !tail.is_empty() && !is_python_container(tail) {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((tail.to_string(), role));
        }
        return;
    }
    if t == "generic_type" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "identifier" {
                    let container = read_text_owned(child, source);
                    if !container.is_empty() && !is_python_container(&container) {
                        let role = if generic {
                            RefRole::Generic
                        } else {
                            RefRole::Direct
                        };
                        out.push((container, role));
                    }
                } else if child.kind() == "type_parameter" {
                    let mut sc = child.walk();
                    if sc.goto_first_child() {
                        loop {
                            if sc.node().is_named() {
                                python_collect_type_refs(sc.node(), source, true, out);
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
        return;
    }
    if t == "subscript" {
        let value = node.child_by_field_name("value");
        if let Some(v) = value {
            python_collect_type_refs(v, source, generic, out);
        }
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if Some(child) != value && child.is_named() {
                    python_collect_type_refs(child, source, true, out);
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
                    python_collect_type_refs(cur.node(), source, generic, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Collect type references from each typed parameter under a `parameters` node.
pub(super) fn python_collect_param_refs(
    params_node: Option<Node<'_>>,
    source: &[u8],
) -> Vec<(String, RefRole)> {
    let mut out = Vec::new();
    let Some(params) = params_node else {
        return out;
    };
    let mut cur = params.walk();
    if !cur.goto_first_child() {
        return out;
    }
    loop {
        let child = cur.node();
        if matches!(child.kind(), "typed_parameter" | "typed_default_parameter")
            && let Some(type_node) = child.child_by_field_name("type")
        {
            python_collect_type_refs(type_node, source, false, &mut out);
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    out
}

// ── C# ────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // single recursive dispatch over tree-sitter C# type kinds
pub(super) fn csharp_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    let t = node.kind();
    if t == "predefined_type" {
        return;
    }
    if t == "identifier" {
        let name = read_text_owned(node, source);
        if !name.is_empty() {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((name, role));
        }
        return;
    }
    if t == "qualified_name" {
        let full = read_text_owned(node, source);
        let tail = full.rsplit('.').next().unwrap_or(&full);
        if !tail.is_empty() {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((tail.to_string(), role));
        }
        return;
    }
    if t == "generic_name" {
        let name_node = node.child_by_field_name("name").or_else(|| {
            let mut sc = node.walk();
            if sc.goto_first_child() {
                loop {
                    if sc.node().kind() == "identifier" {
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
            let name = read_text_owned(nn, source);
            if !name.is_empty() {
                let role = if generic {
                    RefRole::Generic
                } else {
                    RefRole::Direct
                };
                out.push((name, role));
            }
        }
        let mut sc = node.walk();
        if sc.goto_first_child() {
            loop {
                if sc.node().kind() == "type_argument_list" {
                    let mut acur = sc.node().walk();
                    if acur.goto_first_child() {
                        loop {
                            if acur.node().is_named() {
                                csharp_collect_type_refs(acur.node(), source, true, out);
                            }
                            if !acur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                if !sc.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }
    if matches!(
        t,
        "nullable_type" | "array_type" | "pointer_type" | "ref_type"
    ) {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().is_named() {
                    csharp_collect_type_refs(cur.node(), source, generic, out);
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
                    csharp_collect_type_refs(cur.node(), source, generic, out);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Collect attribute names from a C# method's `attribute_list` children.
///
/// `[Authorize, Route("/api")]` on a method produces `["Authorize", "Route"]`.
pub(super) fn csharp_attribute_names(method_node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cur = method_node.walk();
    if !cur.goto_first_child() {
        return names;
    }
    loop {
        let child = cur.node();
        if child.kind() == "attribute_list" {
            let mut acur = child.walk();
            if acur.goto_first_child() {
                loop {
                    let attr = acur.node();
                    if attr.kind() == "attribute" {
                        let name_node = attr.child_by_field_name("name").or_else(|| {
                            let mut sc = attr.walk();
                            if sc.goto_first_child() {
                                loop {
                                    if matches!(sc.node().kind(), "identifier" | "qualified_name") {
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
            }
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    names
}

// ── Java ──────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // single recursive dispatch over tree-sitter Java type kinds
pub(super) fn java_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
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
        if !name.is_empty() {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((name, role));
        }
        return;
    }
    if t == "scoped_type_identifier" {
        let text = read_text_owned(node, source);
        let tail = text.rsplit('.').next().unwrap_or(&text);
        if !tail.is_empty() {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
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
                    if !tail.is_empty() {
                        let role = if generic {
                            RefRole::Generic
                        } else {
                            RefRole::Direct
                        };
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
                                java_collect_type_refs(acur.node(), source, true, out);
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
                    java_collect_type_refs(cur.node(), source, generic, out);
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
                    java_collect_type_refs(cur.node(), source, generic, out);
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

// ── TypeScript / JavaScript ──────────────────────────────────────────────────

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
pub(super) fn ts_collect_type_refs(
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
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
            out.push((name, role));
        }
        return;
    }
    if t == "nested_type_identifier" {
        let text = read_text_owned(node, source);
        let tail = text.rsplit('.').next().unwrap_or(&text);
        if !tail.is_empty() && !is_js_primitive(tail) {
            let role = if generic {
                RefRole::Generic
            } else {
                RefRole::Direct
            };
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
                let role = if generic {
                    RefRole::Generic
                } else {
                    RefRole::Direct
                };
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
                            let role = if generic {
                                RefRole::Generic
                            } else {
                                RefRole::Direct
                            };
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
pub(super) fn ts_heritage_clause_entries(clause: Node<'_>, source: &[u8]) -> Vec<String> {
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

/// Collect annotation names from a Java method's `modifiers` child.
///
/// `@Override @Deprecated public void foo()` yields `["Override", "Deprecated"]`.
pub(super) fn java_method_annotation_names(method_node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let Some(modifiers) = find_modifiers(method_node) else {
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
