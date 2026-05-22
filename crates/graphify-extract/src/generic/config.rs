//! Public type API for the generic tree-sitter extractor.
//!
//! Contains `LangConfig`, `LangId`, and the function-pointer typedefs
//! `ImportHandlerFn` / `ResolveFnNameFn`.  Every language configuration
//! in `crate::lang_configs` is expressed as a `LangConfig` value.

use tree_sitter::{Language, Node};

use crate::types::Edge;

/// Mirrors Python `LanguageConfig` dataclass.
pub struct LangConfig {
    /// tree-sitter language (pre-loaded).
    pub language: Language,

    /// Node types that count as class/type declarations.
    pub class_types: &'static [&'static str],
    /// Node types that count as function/method definitions.
    pub function_types: &'static [&'static str],
    /// Node types that count as import statements.
    pub import_types: &'static [&'static str],
    /// Node types that count as call expressions.
    pub call_types: &'static [&'static str],
    /// Node types for static property access (PHP `Foo::$bar`).
    pub static_prop_types: &'static [&'static str],

    /// Field name for the "name" child on class/function nodes.
    pub name_field: &'static str,
    /// Fallback child types to try when `name_field` is absent.
    pub name_fallback_child_types: &'static [&'static str],
    /// Field name for the "body" child.
    pub body_field: &'static str,
    /// Fallback child types for the body.
    pub body_fallback_child_types: &'static [&'static str],

    /// Field name on a call node for the callee.
    pub call_function_field: &'static str,
    /// Node types for member-access (e.g. `attribute`, `member_expression`).
    pub call_accessor_node_types: &'static [&'static str],
    /// Field name on the accessor node for the method name.
    pub call_accessor_field: &'static str,

    /// Node types that stop call-graph recursion (function boundaries).
    pub function_boundary_types: &'static [&'static str],

    /// Which language module is this config for (affects per-language logic).
    pub lang_id: LangId,

    /// Optional import handler (takes raw node text bytes).
    pub import_handler: Option<ImportHandlerFn>,
    /// Optional function-name resolver (C / C++).
    pub resolve_function_name: Option<ResolveFnNameFn>,

    // ── PHP-specific fields ───────────────────────────────────────────────────
    // These mirror the Python `LanguageConfig` fields added for Laravel/PHP
    // detection. They are empty slices for all non-PHP languages.
    /// PHP: function names that map config keys to graph nodes (e.g. `config()`).
    pub helper_fn_names: &'static [&'static str],
    /// PHP: `IoC` container binding methods (e.g. `bind`, `singleton`).
    pub container_bind_methods: &'static [&'static str],
    /// PHP: property names for event listener arrays (e.g. `$listen`).
    pub event_listener_properties: &'static [&'static str],
}

/// Language discriminant used for per-language special-case logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangId {
    Python,
    JavaScript,
    TypeScript,
    TypeScriptX,
    Java,
    Groovy,
    C,
    Cpp,
    Ruby,
    CSharp,
    Kotlin,
    Scala,
    Php,
    Lua,
    Swift,
    Other,
}

/// Signature for language-specific import-node handlers.
///
/// Receives `(source bytes, node, file_nid, stem, str_path)` and pushes
/// zero or more `Edge` values into `edges`.
pub type ImportHandlerFn = fn(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
);

/// Signature for language-specific function-name resolvers (C / C++).
pub type ResolveFnNameFn = fn(node: Node<'_>, source: &[u8]) -> Option<String>;
