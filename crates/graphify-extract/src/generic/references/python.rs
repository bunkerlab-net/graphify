//! Python type-reference collectors.

use tree_sitter::Node;

use super::RefRole;
use crate::generic::names::read_text_owned;

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
pub(crate) fn python_collect_type_refs(
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
pub(crate) fn python_collect_param_refs(
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
