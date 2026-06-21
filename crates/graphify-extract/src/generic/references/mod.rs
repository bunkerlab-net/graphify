//! Per-language type-reference emitters for function/method declarations.
//!
//! These helpers walk parameter lists, return types, and annotations on
//! function nodes to emit `references` edges with a `context` set to
//! `parameter_type`, `return_type`, `generic_arg`, or `attribute`.
//!
//! Mirrors the `_python_*` / `_csharp_*` / `_java_*` helpers added to
//! `graphify-py/graphify/extract.py` in ab4e542.
//!
//! One submodule per language; this file holds the shared `RefRole`,
//! `RefCollector`, and recursion helpers used across them.

use tree_sitter::Node;

mod c_cpp;
mod csharp;
mod java;
mod kotlin;
mod php;
mod python;
mod scala;
mod swift;
mod ts;

pub(crate) use c_cpp::*;
pub(crate) use csharp::*;
pub(crate) use java::*;
pub(crate) use kotlin::*;
pub(crate) use php::*;
pub(crate) use python::*;
pub(crate) use scala::*;
pub(crate) use swift::*;
pub(crate) use ts::*;

/// Role of a collected type reference. `Direct` = used as the type itself
/// (e.g. `def f(x: Foo)`), `Generic` = used as a type argument to a generic
/// (e.g. `def f(x: list[Foo])`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefRole {
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
