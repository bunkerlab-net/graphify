//! Pre-built `LangConfig` instances for each supported language.
//!
//! Mirrors the Python `_PYTHON_CONFIG`, `_JS_CONFIG`, etc. module-level constants.
//!
//! API notes:
//!  - Most crates expose `LANGUAGE: LanguageFn` → call `.into()` for `tree_sitter::Language`.
//!  - `tree-sitter-kotlin` exposes `language() -> Language` directly.
//!  - `tree-sitter-swift` exposes `LANGUAGE: LanguageFn`.
//!  - PHP uses `LANGUAGE_PHP`.
//!  - TypeScript uses `LANGUAGE_TYPESCRIPT` / `LANGUAGE_TSX`.

use std::sync::LazyLock;

use crate::generic::{LangConfig, LangId, get_c_func_name, get_cpp_func_name};
use crate::import_handlers::{
    import_c, import_csharp, import_java, import_js, import_kotlin, import_lua, import_php,
    import_python, import_scala, import_swift,
};

// ── Python ────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for Python, using tree-sitter-python.
pub static PYTHON: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_python::LANGUAGE.into(),
    class_types: &["class_definition"],
    function_types: &["function_definition"],
    import_types: &["import_statement", "import_from_statement"],
    call_types: &["call"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "function",
    call_accessor_node_types: &["attribute"],
    call_accessor_field: "attribute",
    function_boundary_types: &["function_definition"],
    lang_id: LangId::Python,
    import_handler: Some(import_python),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── JavaScript ────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for JavaScript (`.js`, `.jsx`, `.mjs`), using tree-sitter-javascript.
pub static JAVASCRIPT: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_javascript::LANGUAGE.into(),
    class_types: &["class_declaration"],
    function_types: &["function_declaration", "method_definition"],
    // `export_statement` is intentionally treated as import-like so the JS
    // import handler can resolve re-exports — both `export { x } from '...'`
    // and `export * from '...'` reach module specifiers that need cross-file
    // edge creation, just like a regular `import`.
    import_types: &["import_statement", "export_statement"],
    call_types: &["call_expression", "new_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "function",
    call_accessor_node_types: &["member_expression"],
    call_accessor_field: "property",
    function_boundary_types: &[
        "function_declaration",
        "arrow_function",
        "method_definition",
    ],
    lang_id: LangId::JavaScript,
    import_handler: Some(import_js),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── TypeScript ────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for TypeScript (`.ts`), using `LANGUAGE_TYPESCRIPT`.
pub static TYPESCRIPT: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    class_types: &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "type_alias_declaration",
    ],
    function_types: &["function_declaration", "method_definition"],
    // `export_statement` is intentionally treated as import-like so the JS
    // import handler can resolve re-exports — both `export { x } from '...'`
    // and `export * from '...'` reach module specifiers that need cross-file
    // edge creation, just like a regular `import`.
    import_types: &["import_statement", "export_statement"],
    call_types: &["call_expression", "new_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "function",
    call_accessor_node_types: &["member_expression"],
    call_accessor_field: "property",
    function_boundary_types: &[
        "function_declaration",
        "arrow_function",
        "method_definition",
    ],
    lang_id: LangId::TypeScript,
    import_handler: Some(import_js),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── TypeScript (TSX) ──────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for TypeScript JSX (`.tsx`), using `LANGUAGE_TSX`.
pub static TYPESCRIPT_TSX: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_typescript::LANGUAGE_TSX.into(),
    class_types: &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "type_alias_declaration",
    ],
    function_types: &["function_declaration", "method_definition"],
    // `export_statement` is intentionally treated as import-like so the JS
    // import handler can resolve re-exports — both `export { x } from '...'`
    // and `export * from '...'` reach module specifiers that need cross-file
    // edge creation, just like a regular `import`.
    import_types: &["import_statement", "export_statement"],
    call_types: &["call_expression", "new_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "function",
    call_accessor_node_types: &["member_expression"],
    call_accessor_field: "property",
    function_boundary_types: &[
        "function_declaration",
        "arrow_function",
        "method_definition",
    ],
    lang_id: LangId::TypeScriptX,
    import_handler: Some(import_js),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── Java ──────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for Java, using tree-sitter-java.
pub static JAVA: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_java::LANGUAGE.into(),
    class_types: &["class_declaration", "interface_declaration"],
    function_types: &["method_declaration", "constructor_declaration"],
    import_types: &["import_declaration"],
    call_types: &["method_invocation"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "name",
    call_accessor_node_types: &[],
    call_accessor_field: "",
    function_boundary_types: &["method_declaration", "constructor_declaration"],
    lang_id: LangId::Java,
    import_handler: Some(import_java),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── Groovy ────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for Groovy (including Spock specs), using tree-sitter-groovy.
pub static GROOVY: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_groovy::LANGUAGE.into(),
    class_types: &["class_declaration", "interface_declaration"],
    function_types: &["method_declaration", "constructor_declaration"],
    import_types: &["import_declaration"],
    call_types: &["method_invocation"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "name",
    call_accessor_node_types: &[],
    call_accessor_field: "",
    function_boundary_types: &["method_declaration", "constructor_declaration"],
    lang_id: LangId::Groovy,
    import_handler: Some(import_java),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── C ─────────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for C, using tree-sitter-c.
pub static C: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_c::LANGUAGE.into(),
    class_types: &[],
    function_types: &["function_definition"],
    import_types: &["preproc_include"],
    call_types: &["call_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "function",
    call_accessor_node_types: &["field_expression"],
    call_accessor_field: "field",
    function_boundary_types: &["function_definition"],
    lang_id: LangId::C,
    import_handler: Some(import_c),
    resolve_function_name: Some(get_c_func_name),
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── C++ ───────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for C++, using tree-sitter-cpp.
pub static CPP: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_cpp::LANGUAGE.into(),
    class_types: &["class_specifier", "struct_specifier"],
    function_types: &["function_definition"],
    import_types: &["preproc_include"],
    call_types: &["call_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &[],
    call_function_field: "function",
    call_accessor_node_types: &["field_expression", "qualified_identifier"],
    call_accessor_field: "field",
    function_boundary_types: &["function_definition"],
    lang_id: LangId::Cpp,
    import_handler: Some(import_c),
    resolve_function_name: Some(get_cpp_func_name),
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── Ruby ──────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for Ruby, using tree-sitter-ruby.
pub static RUBY: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_ruby::LANGUAGE.into(),
    class_types: &["class"],
    function_types: &["method", "singleton_method"],
    import_types: &[],
    call_types: &["call"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &["constant", "scope_resolution", "identifier"],
    body_field: "body",
    body_fallback_child_types: &["body_statement"],
    call_function_field: "method",
    call_accessor_node_types: &[],
    call_accessor_field: "",
    function_boundary_types: &["method", "singleton_method"],
    lang_id: LangId::Other,
    import_handler: None,
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── C# ────────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for C#, using tree-sitter-c-sharp.
pub static CSHARP: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_c_sharp::LANGUAGE.into(),
    class_types: &["class_declaration", "interface_declaration"],
    function_types: &["method_declaration"],
    import_types: &["using_directive"],
    call_types: &["invocation_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &[],
    body_field: "body",
    body_fallback_child_types: &["declaration_list"],
    call_function_field: "function",
    call_accessor_node_types: &["member_access_expression"],
    call_accessor_field: "name",
    function_boundary_types: &["method_declaration"],
    lang_id: LangId::CSharp,
    import_handler: Some(import_csharp),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── Kotlin ────────────────────────────────────────────────────────────────────
// Uses tree-sitter-kotlin-ng which targets tree-sitter 0.23+.

/// Pre-built [`LangConfig`] for Kotlin, using tree-sitter-kotlin-ng.
pub static KOTLIN: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_kotlin_ng::LANGUAGE.into(),
    class_types: &["class_declaration", "object_declaration"],
    function_types: &["function_declaration"],
    import_types: &["import_header"],
    call_types: &["call_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &["simple_identifier", "identifier"],
    body_field: "body",
    body_fallback_child_types: &["function_body", "class_body"],
    call_function_field: "",
    call_accessor_node_types: &["navigation_expression"],
    call_accessor_field: "",
    function_boundary_types: &["function_declaration"],
    lang_id: LangId::Kotlin,
    import_handler: Some(import_kotlin),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── Scala ─────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for Scala, using tree-sitter-scala.
pub static SCALA: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_scala::LANGUAGE.into(),
    class_types: &["class_definition", "object_definition"],
    function_types: &["function_definition"],
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &["identifier"],
    body_field: "body",
    body_fallback_child_types: &["template_body"],
    call_function_field: "",
    call_accessor_node_types: &["field_expression"],
    call_accessor_field: "field",
    function_boundary_types: &["function_definition"],
    lang_id: LangId::Scala,
    import_handler: Some(import_scala),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── PHP ───────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for PHP (including Laravel), using `LANGUAGE_PHP`.
pub static PHP: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_php::LANGUAGE_PHP.into(),
    class_types: &["class_declaration"],
    function_types: &["function_definition", "method_declaration"],
    import_types: &["namespace_use_clause"],
    call_types: &[
        "function_call_expression",
        "member_call_expression",
        "scoped_call_expression",
        "class_constant_access_expression",
    ],
    static_prop_types: &["scoped_property_access_expression"],
    name_field: "name",
    name_fallback_child_types: &["name"],
    body_field: "body",
    body_fallback_child_types: &["declaration_list", "compound_statement"],
    call_function_field: "function",
    call_accessor_node_types: &["member_call_expression"],
    call_accessor_field: "name",
    function_boundary_types: &["function_definition", "method_declaration"],
    lang_id: LangId::Php,
    import_handler: Some(import_php),
    resolve_function_name: None,
    helper_fn_names: &["config", "view", "route"],
    container_bind_methods: &["bind", "singleton", "scoped", "instance"],
    event_listener_properties: &["listen", "subscribe"],
});

// ── Lua ───────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for Lua, using tree-sitter-lua.
pub static LUA: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_lua::LANGUAGE.into(),
    class_types: &[],
    function_types: &["function_declaration"],
    import_types: &["variable_declaration"],
    call_types: &["function_call"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &["identifier", "method_index_expression"],
    body_field: "body",
    body_fallback_child_types: &["block"],
    call_function_field: "name",
    call_accessor_node_types: &["method_index_expression"],
    call_accessor_field: "name",
    function_boundary_types: &["function_declaration"],
    lang_id: LangId::Other,
    import_handler: Some(import_lua),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});

// ── Swift ─────────────────────────────────────────────────────────────────────

/// Pre-built [`LangConfig`] for Swift, using tree-sitter-swift.
pub static SWIFT: LazyLock<LangConfig> = LazyLock::new(|| LangConfig {
    language: tree_sitter_swift::LANGUAGE.into(),
    class_types: &["class_declaration", "protocol_declaration"],
    function_types: &[
        "function_declaration",
        "init_declaration",
        "deinit_declaration",
        "subscript_declaration",
    ],
    import_types: &["import_declaration"],
    call_types: &["call_expression"],
    static_prop_types: &[],
    name_field: "name",
    name_fallback_child_types: &["simple_identifier", "type_identifier", "user_type"],
    body_field: "body",
    body_fallback_child_types: &[
        "class_body",
        "protocol_body",
        "function_body",
        "enum_class_body",
    ],
    call_function_field: "",
    call_accessor_node_types: &["navigation_expression"],
    call_accessor_field: "",
    function_boundary_types: &[
        "function_declaration",
        "init_declaration",
        "deinit_declaration",
        "subscript_declaration",
    ],
    lang_id: LangId::Swift,
    import_handler: Some(import_swift),
    resolve_function_name: None,
    helper_fn_names: &[],
    container_bind_methods: &[],
    event_listener_properties: &[],
});
