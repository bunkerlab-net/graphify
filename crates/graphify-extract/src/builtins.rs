//! Language built-in globals that the AST may classify as call targets when
//! used as constructors or coercion functions (e.g. `String(x)`, `Number(x)`,
//! `len(x)`).
//!
//! Without filtering they become god-nodes accumulating spurious edges from
//! every call site. The filter is applied only to *unresolved* unqualified
//! calls during resolution, across all languages. The set is multi-language
//! (JavaScript/TypeScript ECMAScript + browser/Node globals, plus Python
//! built-in callables) and mirrors Python `_LANGUAGE_BUILTIN_GLOBALS`, which
//! is likewise multi-language (issue #726).

use std::collections::HashSet;
use std::sync::LazyLock;

static LANGUAGE_BUILTIN_GLOBALS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // JavaScript / TypeScript ECMAScript built-ins
        "String",
        "Number",
        "Boolean",
        "Object",
        "Array",
        "Symbol",
        "BigInt",
        "Date",
        "RegExp",
        "Error",
        "TypeError",
        "RangeError",
        "SyntaxError",
        "ReferenceError",
        "EvalError",
        "URIError",
        "Promise",
        "Map",
        "Set",
        "WeakMap",
        "WeakSet",
        "JSON",
        "Math",
        "Reflect",
        "Proxy",
        "Intl",
        "parseInt",
        "parseFloat",
        "isNaN",
        "isFinite",
        "encodeURIComponent",
        "decodeURIComponent",
        "encodeURI",
        "decodeURI",
        // Browser / Node common globals
        "URL",
        "URLSearchParams",
        "FormData",
        "Blob",
        "File",
        "Headers",
        "Request",
        "Response",
        "AbortController",
        "AbortSignal",
        "TextEncoder",
        "TextDecoder",
        "console",
        // Python built-in callables
        "str",
        "int",
        "float",
        "bool",
        "list",
        "dict",
        "set",
        "tuple",
        "bytes",
        "len",
        "range",
        "enumerate",
        "zip",
        "map",
        "filter",
        "sum",
        "min",
        "max",
        "print",
        "open",
        "isinstance",
        "type",
        "super",
        "sorted",
        "reversed",
        "any",
        "all",
        "abs",
        "round",
        "next",
        "iter",
        "hash",
        "id",
        "repr",
        "callable",
        "getattr",
        "setattr",
        "hasattr",
        "delattr",
        "vars",
        "dir",
    ]
    .into_iter()
    .collect()
});

/// Return `true` when `name` is a language built-in global that should not be
/// resolved as a call target. Matching is case-sensitive against the raw
/// callee text (mirrors Python's `callee in _LANGUAGE_BUILTIN_GLOBALS`).
#[must_use]
pub(crate) fn is_language_builtin_global(name: &str) -> bool {
    LANGUAGE_BUILTIN_GLOBALS.contains(name)
}
