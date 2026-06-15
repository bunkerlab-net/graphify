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
use super::walk::first_child_kind;

/// Role of a collected type reference. `Direct` = used as the type itself
/// (e.g. `def f(x: Foo)`), `Generic` = used as a type argument to a generic
/// (e.g. `def f(x: list[Foo])`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Scalar builtins and `unittest.mock` names that appear as type annotations but
/// carry no useful semantic meaning as graph nodes (#1147). Suppressed at the
/// annotation walker level so they are never created as nodes or emitted as
/// edges. Mirrors `_PYTHON_ANNOTATION_NOISE` in `extract.py`.
const PYTHON_ANNOTATION_NOISE: &[&str] = &[
    // scalar builtins
    "str",
    "int",
    "float",
    "bool",
    "bytes",
    "bytearray",
    "complex",
    "object",
    "True",
    "False",
    // unittest.mock
    "MagicMock",
    "Mock",
    "AsyncMock",
    "NonCallableMock",
    "NonCallableMagicMock",
    "PropertyMock",
    "patch",
    "sentinel",
];

fn is_python_container(name: &str) -> bool {
    PYTHON_TYPE_CONTAINERS.contains(&name)
}

fn is_python_annotation_noise(name: &str) -> bool {
    PYTHON_ANNOTATION_NOISE.contains(&name)
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
        if !name.is_empty() && !is_python_container(&name) && !is_python_annotation_noise(&name) {
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
        if !tail.is_empty() && !is_python_container(tail) && !is_python_annotation_noise(tail) {
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
                    if !container.is_empty()
                        && !is_python_container(&container)
                        && !is_python_annotation_noise(&container)
                    {
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
#[must_use]
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
#[must_use]
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
#[must_use]
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
#[must_use]
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

// ── Shared helpers for the v0.8.25 cross-language type-ref collectors ──────────

/// Map a `generic` flag to the corresponding [`RefRole`].
fn role_of(generic: bool) -> RefRole {
    if generic {
        RefRole::Generic
    } else {
        RefRole::Direct
    }
}

/// A language type-reference collector: walks a type node, appending
/// `(name, role)` tuples for each referenced user type.
pub(super) type RefCollector = fn(Node<'_>, &[u8], bool, &mut Vec<(String, RefRole)>);

/// Recurse `collect` over every named child of `node`, preserving `generic`.
fn recurse_named_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
    collect: RefCollector,
) {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().is_named() {
                collect(cur.node(), source, generic, out);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

// ── Swift ───────────────────────────────────────────────────────────────────

/// Return the head `type_identifier` text from a Swift `user_type` node.
#[must_use]
pub(super) fn swift_user_type_name(user_type_node: Node<'_>, source: &[u8]) -> Option<String> {
    first_child_kind(user_type_node, "type_identifier")
        .map(|n| read_text_owned(n, source))
        .filter(|t| !t.is_empty())
}

/// Return the `type_annotation` child of a Swift `property_declaration`, if any.
#[must_use]
pub(super) fn swift_property_type_node(property_node: Node<'_>) -> Option<Node<'_>> {
    first_child_kind(property_node, "type_annotation")
}

/// Walk a Swift type expression; append `(name, role)` tuples. Mirrors
/// Python `_swift_collect_type_refs`.
pub(super) fn swift_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "user_type" => {
            if let Some(head) = first_child_kind(node, "type_identifier") {
                let text = read_text_owned(head, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        recurse_named_refs(cur.node(), source, true, out, swift_collect_type_refs);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        // `optional_type`, `array_type`, `dictionary_type`, `tuple_type`, etc.
        // are all named wrappers handled identically by the fallback below.
        _ if node.is_named() => {
            recurse_named_refs(node, source, generic, out, swift_collect_type_refs);
        }
        _ => {}
    }
}

// ── PHP ───────────────────────────────────────────────────────────────────────

/// Return the unqualified tail of a PHP `name` / `qualified_name` node.
#[must_use]
pub(super) fn php_name_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let full = read_text_owned(node, source);
    let tail = full.rsplit('\\').next().unwrap_or(&full);
    if tail.is_empty() {
        None
    } else {
        Some(tail.to_string())
    }
}

/// PHP type-node kinds that count as a type annotation on params/properties.
pub(super) const PHP_TYPE_NODE_KINDS: &[&str] = &[
    "named_type",
    "primitive_type",
    "nullable_type",
    "union_type",
    "intersection_type",
    "optional_type",
];

/// Return the return-type node following `formal_parameters` on a PHP method.
#[must_use]
pub(super) fn php_method_return_type_node(method_node: Node<'_>) -> Option<Node<'_>> {
    let mut saw_params = false;
    let mut cur = method_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "formal_parameters" {
                saw_params = true;
            } else if saw_params
                && c.is_named()
                && c.kind() != "compound_statement"
                && PHP_TYPE_NODE_KINDS.contains(&c.kind())
            {
                return Some(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Walk a PHP type expression; append `(name, role)` tuples. Mirrors
/// Python `_php_collect_type_refs`.
pub(super) fn php_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "primitive_type" => {}
        "named_type" => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if matches!(cur.node().kind(), "name" | "qualified_name") {
                        if let Some(text) = php_name_text(cur.node(), source) {
                            out.push((text, role_of(generic)));
                        }
                        return;
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "name" | "qualified_name" => {
            if let Some(text) = php_name_text(node, source) {
                out.push((text, role_of(generic)));
            }
        }
        // `nullable_type` / `union_type` / `intersection_type` / `optional_type`
        // are named wrappers handled identically by the fallback below.
        _ if node.is_named() => {
            recurse_named_refs(node, source, generic, out, php_collect_type_refs);
        }
        _ => {}
    }
}

// ── Kotlin ────────────────────────────────────────────────────────────────────

/// Return the head identifier text from a Kotlin `user_type` node.
#[must_use]
pub(super) fn kotlin_user_type_name(user_type_node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cur = user_type_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            match c.kind() {
                "type_identifier" | "identifier" => {
                    let text = read_text_owned(c, source);
                    return if text.is_empty() { None } else { Some(text) };
                }
                "simple_user_type" => {
                    if let Some(sub) = first_named_identifier(c) {
                        let text = read_text_owned(sub, source);
                        return if text.is_empty() { None } else { Some(text) };
                    }
                }
                _ => {}
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Return the first `identifier` / `type_identifier` child of `node`.
fn first_named_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if matches!(cur.node().kind(), "identifier" | "type_identifier") {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Find the type node within a Kotlin `property_declaration`.
#[must_use]
pub(super) fn kotlin_property_type_node(property_node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = property_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "variable_declaration"
                && let Some(sub) = kotlin_type_child(c)
            {
                return Some(sub);
            }
            if matches!(c.kind(), "user_type" | "nullable_type" | "type_reference") {
                return Some(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn kotlin_type_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if matches!(
                cur.node().kind(),
                "user_type" | "nullable_type" | "type_reference"
            ) {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Find the return-type node of a Kotlin `function_declaration`.
#[must_use]
pub(super) fn kotlin_function_return_type_node(func_node: Node<'_>) -> Option<Node<'_>> {
    let mut saw_params = false;
    let mut saw_colon = false;
    let mut cur = func_node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if c.kind() == "function_value_parameters" {
                saw_params = true;
            } else if saw_params && c.kind() == ":" {
                saw_colon = true;
            } else if saw_colon && c.is_named() {
                return Some(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Walk a Kotlin type expression; append `(name, role)` tuples. Mirrors
/// Python `_kotlin_collect_type_refs`.
pub(super) fn kotlin_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "integral_literal" | "boolean_literal" => {}
        "user_type" => {
            if let Some(head) = kotlin_user_type_head(node) {
                let text = read_text_owned(head, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            kotlin_collect_type_arguments(node, source, out);
        }
        "identifier" | "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        // `nullable_type` / `parenthesized_type` / `type_reference` are named
        // wrappers handled identically by the fallback below.
        _ if node.is_named() => {
            recurse_named_refs(node, source, generic, out, kotlin_collect_type_refs);
        }
        _ => {}
    }
}

/// Return the head `identifier`/`type_identifier` node of a Kotlin `user_type`,
/// drilling through a `simple_user_type` wrapper.
fn kotlin_user_type_head(node: Node<'_>) -> Option<Node<'_>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if matches!(c.kind(), "identifier" | "type_identifier") {
                return Some(c);
            }
            if c.kind() == "simple_user_type" {
                return first_named_identifier(c);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Recurse into a Kotlin `user_type`'s `type_arguments`, marking refs generic.
fn kotlin_collect_type_arguments(node: Node<'_>, source: &[u8], out: &mut Vec<(String, RefRole)>) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        if cur.node().kind() == "type_arguments" {
            let mut acur = cur.node().walk();
            if acur.goto_first_child() {
                loop {
                    let arg = acur.node();
                    if arg.kind() == "type_projection" {
                        recurse_named_refs(arg, source, true, out, kotlin_collect_type_refs);
                    } else if arg.is_named() {
                        kotlin_collect_type_refs(arg, source, true, out);
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

// ── Scala ─────────────────────────────────────────────────────────────────────

/// Walk a Scala type expression; append `(name, role)` tuples. Mirrors
/// Python `_scala_collect_type_refs`.
pub(super) fn scala_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    match node.kind() {
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        "generic_type" => {
            let base = node
                .child_by_field_name("type")
                .or_else(|| first_child_kind(node, "type_identifier"));
            if let Some(base) = base
                && base.kind() == "type_identifier"
            {
                let text = read_text_owned(base, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    if cur.node().kind() == "type_arguments" {
                        recurse_named_refs(cur.node(), source, true, out, scala_collect_type_refs);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "compound_type" | "infix_type" | "function_type" | "tuple_type" | "annotated_type"
        | "projected_type" => {
            recurse_named_refs(node, source, generic, out, scala_collect_type_refs);
        }
        // No catch-all recurse: graphify-py's `_scala_collect_type_refs`
        // (extract.py) handles only `type_identifier`, `generic_type`, and the
        // wrapper kinds above, so other named nodes are intentionally ignored to
        // preserve parity.
        _ => {}
    }
}

// ── C / C++ ─────────────────────────────────────────────────────────────────

/// Node kinds that are C/C++ primitive types and never yield a type reference.
const C_PRIMITIVE_TYPE_NODES: &[&str] = &[
    "primitive_type",
    "sized_type_specifier",
    "auto",
    "placeholder_type_specifier",
];

/// Walk a C type expression; append `(name, role)` tuples for user-defined types.
pub(super) fn c_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    if C_PRIMITIVE_TYPE_NODES.contains(&node.kind()) {
        return;
    }
    match node.kind() {
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        "pointer_declarator"
        | "reference_declarator"
        | "array_declarator"
        | "type_qualifier"
        | "type_descriptor"
        | "abstract_pointer_declarator"
        | "abstract_reference_declarator"
        | "abstract_array_declarator" => {
            recurse_named_refs(node, source, generic, out, c_collect_type_refs);
        }
        _ => {}
    }
}

/// Walk a C++ type expression; append `(name, role)` tuples. Resolves
/// `qualified_identifier` tails and `template_type` base + arguments.
pub(super) fn cpp_collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    out: &mut Vec<(String, RefRole)>,
) {
    if C_PRIMITIVE_TYPE_NODES.contains(&node.kind()) {
        return;
    }
    match node.kind() {
        "type_identifier" => {
            let text = read_text_owned(node, source);
            if !text.is_empty() {
                out.push((text, role_of(generic)));
            }
        }
        "qualified_identifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                cpp_collect_type_refs(name_node, source, generic, out);
            }
        }
        "template_type" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let text = read_text_owned(name_node, source);
                if !text.is_empty() {
                    out.push((text, role_of(generic)));
                }
            }
            if let Some(args_node) = node.child_by_field_name("arguments") {
                recurse_named_refs(args_node, source, true, out, cpp_collect_type_refs);
            }
        }
        "type_descriptor"
        | "pointer_declarator"
        | "reference_declarator"
        | "array_declarator"
        | "type_qualifier"
        | "abstract_pointer_declarator"
        | "abstract_reference_declarator"
        | "abstract_array_declarator" => {
            recurse_named_refs(node, source, generic, out, cpp_collect_type_refs);
        }
        _ => {}
    }
}
