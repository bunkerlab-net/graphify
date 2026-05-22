//! Generic tree-sitter extractor driven by `LangConfig`.
//!
//! Mirrors the Python `_extract_generic()` function and the `walk` /
//! `walk_calls` inner functions.

// Tree-sitter row numbers represent source line indices; files with 2^32+
// lines do not exist in practice, so usize→u32 truncation is safe.
#![allow(clippy::cast_possible_truncation)]

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Language, Node, Parser};

use crate::ids::{file_stem, make_id, make_id1};
use crate::tsconfig::load_tsconfig_aliases;
use crate::types::{Edge, FileResult, Node as GNode, RawCall};

// ── Language config ───────────────────────────────────────────────────────────

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
    /// Optional function-name resolver (C/C++ declarator unwrapping).
    pub resolve_function_name: Option<ResolveFnNameFn>,
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

// ── Node/edge helpers ─────────────────────────────────────────────────────────

fn add_node(
    nid: &str,
    label: &str,
    line: u32,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) {
    if seen_ids.insert(nid.to_string()) {
        nodes.push(GNode {
            id: nid.to_string(),
            label: label.to_string(),
            file_type: "code".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
        });
    }
}

fn add_edge(
    src: &str,
    tgt: &str,
    relation: &str,
    line: u32,
    str_path: &str,
    context: Option<&str>,
    edges: &mut Vec<Edge>,
) {
    edges.push(Edge {
        source: src.to_string(),
        target: tgt.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: context.map(str::to_string),
        confidence_score: None,
    });
}

fn read_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

fn read_text_owned(node: Node<'_>, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).into_owned()
}

fn find_body<'tree>(node: Node<'tree>, config: &LangConfig) -> Option<Node<'tree>> {
    if let Some(b) = node.child_by_field_name(config.body_field) {
        return Some(b);
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if config.body_fallback_child_types.contains(&child.kind()) {
                return Some(child);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

// ── C/C++ function-name helpers ───────────────────────────────────────────────

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

// ── Read C# type name ─────────────────────────────────────────────────────────

fn read_csharp_type_name(node: Option<Node<'_>>, source: &[u8]) -> Option<String> {
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

// ── ensure_named_node ─────────────────────────────────────────────────────────

fn ensure_named_node(
    name: &str,
    line: u32,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) -> String {
    let nid1 = make_id(&[stem, name]);
    if seen_ids.contains(&nid1) {
        return nid1;
    }
    let nid2 = make_id1(name);
    if !seen_ids.contains(&nid2) {
        add_node(&nid2, name, line, str_path, nodes, seen_ids);
    }
    nid2
}

// ── walk (structural pass) ────────────────────────────────────────────────────

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn walk<'tree>(
    node: Node<'tree>,
    parent_class_nid: Option<&str>,
    source: &[u8],
    config: &LangConfig,
    file_nid: &str,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, Node<'tree>)>,
) {
    let t = node.kind();

    // ── Imports ──────────────────────────────────────────────────────────────
    if config.import_types.contains(&t) {
        if let Some(handler) = config.import_handler {
            handler(source, node, file_nid, stem, str_path, edges);
        }
        return;
    }

    // ── Classes ──────────────────────────────────────────────────────────────
    if config.class_types.contains(&t) {
        // Resolve class name
        let name_node = node.child_by_field_name(config.name_field).or_else(|| {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if config.name_fallback_child_types.contains(&child.kind()) {
                        return Some(child);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        });
        let Some(name_node) = name_node else { return };
        let class_name = read_text_owned(name_node, source);
        let class_nid = make_id(&[stem, &class_name]);
        let line = node.start_position().row as u32 + 1;
        add_node(&class_nid, &class_name, line, str_path, nodes, seen_ids);
        add_edge(
            file_nid, &class_nid, "contains", line, str_path, None, edges,
        );

        // Python inheritance
        if config.lang_id == LangId::Python
            && let Some(args) = node.child_by_field_name("superclasses")
        {
            let mut cur = args.walk();
            if cur.goto_first_child() {
                loop {
                    let arg = cur.node();
                    if arg.kind() == "identifier" {
                        let base = read_text_owned(arg, source);
                        let base_nid = if seen_ids.contains(&make_id(&[stem, &base])) {
                            make_id(&[stem, &base])
                        } else {
                            let bn = make_id1(&base);
                            if !seen_ids.contains(&bn) {
                                nodes.push(GNode {
                                    id: bn.clone(),
                                    label: base.clone(),
                                    file_type: "code".to_string(),
                                    source_file: String::new(),
                                    source_location: None,
                                });
                                seen_ids.insert(bn.clone());
                            }
                            bn
                        };
                        add_edge(
                            &class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        // Swift conformance/inheritance
        if config.lang_id == LangId::Swift {
            emit_swift_inheritance(
                node, source, &class_nid, line, stem, str_path, nodes, edges, seen_ids,
            );
        }

        // C# base_list
        if config.lang_id == LangId::CSharp {
            emit_csharp_inheritance(
                node, source, &class_nid, line, stem, str_path, nodes, edges, seen_ids,
            );
        }

        // Java extends/implements
        if config.lang_id == LangId::Java {
            emit_java_inheritance(
                node, source, &class_nid, t, line, stem, str_path, nodes, edges, seen_ids,
            );
        }

        // C++ base_class_clause
        if config.lang_id == LangId::Cpp {
            emit_cpp_inheritance(
                node, source, &class_nid, line, stem, str_path, nodes, edges, seen_ids,
            );
        }

        // Find body and recurse
        if let Some(body) = find_body(node, config) {
            let mut cur = body.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    walk(
                        child,
                        Some(&class_nid),
                        source,
                        config,
                        file_nid,
                        stem,
                        str_path,
                        nodes,
                        edges,
                        seen_ids,
                        function_bodies,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        return;
    }

    // ── PHP event listener property arrays ───────────────────────────────────
    // (skip for brevity in generic pass — handled in PHP-specific extractor)

    // ── C# field_declaration ─────────────────────────────────────────────────
    if config.lang_id == LangId::CSharp
        && t == "field_declaration"
        && let Some(parent) = parent_class_nid
    {
        let type_node = node.child_by_field_name("type").or_else(|| {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "variable_declaration" {
                        return child.child_by_field_name("type");
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            None
        });
        if let Some(type_name) = read_csharp_type_name(type_node, source)
            && !type_name.is_empty()
        {
            let line = node.start_position().row as u32 + 1;
            let tgt = ensure_named_node(&type_name, line, stem, str_path, nodes, seen_ids);
            let e = Edge {
                source: parent.to_string(),
                target: tgt,
                relation: "references".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: Some("field".to_string()),
                confidence_score: None,
            };
            edges.push(e);
        }
        return;
    }

    // ── C++ field_declaration ─────────────────────────────────────────────────
    if config.lang_id == LangId::Cpp
        && t == "field_declaration"
        && let Some(parent) = parent_class_nid
    {
        let mut cur = node.walk();
        // children_by_field_name("declarator")
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if (child.kind() == "field_identifier"
                    || matches!(
                        child.kind(),
                        "pointer_declarator"
                            | "reference_declarator"
                            | "init_declarator"
                            | "array_declarator"
                            | "identifier"
                    ))
                    && let Some(name) = get_cpp_func_name(child, source)
                {
                    let line = child.start_position().row as u32 + 1;
                    let field_nid = make_id(&[parent, &name]);
                    add_node(&field_nid, &name, line, str_path, nodes, seen_ids);
                    let e = Edge {
                        source: parent.to_string(),
                        target: field_nid,
                        relation: "defines".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: Some("field".to_string()),
                        confidence_score: None,
                    };
                    edges.push(e);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // ── Functions ─────────────────────────────────────────────────────────────
    if config.function_types.contains(&t) {
        let func_name: Option<String> = match t {
            "deinit_declaration" => Some("deinit".to_string()),
            "subscript_declaration" => Some("subscript".to_string()),
            _ if let Some(resolver) = config.resolve_function_name => node
                .child_by_field_name("declarator")
                .and_then(|d| resolver(d, source)),
            _ => {
                let nn = node.child_by_field_name(config.name_field).or_else(|| {
                    let mut cur = node.walk();
                    if cur.goto_first_child() {
                        loop {
                            let child = cur.node();
                            if config.name_fallback_child_types.contains(&child.kind()) {
                                return Some(child);
                            }
                            if !cur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                    None
                });
                nn.map(|n| read_text_owned(n, source))
            }
        };

        let Some(func_name) = func_name else { return };
        if func_name.is_empty() {
            return;
        }

        let line = node.start_position().row as u32 + 1;
        let (func_nid, label, parent_nid) = if let Some(parent) = parent_class_nid {
            (
                make_id(&[parent, &func_name]),
                format!(".{func_name}()"),
                parent.to_string(),
            )
        } else {
            (
                make_id(&[stem, &func_name]),
                format!("{func_name}()"),
                file_nid.to_string(),
            )
        };

        add_node(&func_nid, &label, line, str_path, nodes, seen_ids);
        let relation = if parent_class_nid.is_some() {
            "method"
        } else {
            "contains"
        };
        add_edge(
            &parent_nid,
            &func_nid,
            relation,
            line,
            str_path,
            None,
            edges,
        );

        if let Some(body) = find_body(node, config) {
            function_bodies.push((func_nid, body));
        }
        return;
    }

    // ── JS/TS extra walk (arrow functions, CJS require) ───────────────────────
    if (config.lang_id == LangId::JavaScript
        || config.lang_id == LangId::TypeScript
        || config.lang_id == LangId::TypeScriptX)
        && js_extra_walk(
            node,
            source,
            config,
            file_nid,
            stem,
            str_path,
            nodes,
            edges,
            seen_ids,
            function_bodies,
            parent_class_nid,
        )
    {
        return;
    }

    // ── C# namespace_declaration ──────────────────────────────────────────────
    if config.lang_id == LangId::CSharp && t == "namespace_declaration" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let ns_name = read_text_owned(name_node, source);
            let ns_nid = make_id(&[stem, &ns_name]);
            let line = node.start_position().row as u32 + 1;
            add_node(&ns_nid, &ns_name, line, str_path, nodes, seen_ids);
            add_edge(file_nid, &ns_nid, "contains", line, str_path, None, edges);
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut cur = body.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    walk(
                        child,
                        parent_class_nid,
                        source,
                        config,
                        file_nid,
                        stem,
                        str_path,
                        nodes,
                        edges,
                        seen_ids,
                        function_bodies,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        return;
    }

    // ── Swift enum_entry ──────────────────────────────────────────────────────
    if config.lang_id == LangId::Swift
        && t == "enum_entry"
        && let Some(parent) = parent_class_nid
    {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "simple_identifier" {
                    let case_name = read_text_owned(child, source);
                    let case_nid = make_id(&[parent, &case_name]);
                    let line = node.start_position().row as u32 + 1;
                    add_node(&case_nid, &case_name, line, str_path, nodes, seen_ids);
                    add_edge(parent, &case_nid, "case_of", line, str_path, None, edges);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }

    // ── Default: recurse ──────────────────────────────────────────────────────
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            walk(
                child,
                None,
                source,
                config,
                file_nid,
                stem,
                str_path,
                nodes,
                edges,
                seen_ids,
                function_bodies,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

// ── JS/TS extra walk ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn js_extra_walk<'tree>(
    node: Node<'tree>,
    source: &[u8],
    _config: &LangConfig,
    file_nid: &str,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
    function_bodies: &mut Vec<(String, Node<'tree>)>,
    _parent_class_nid: Option<&str>,
) -> bool {
    let t = node.kind();
    if t != "lexical_declaration" && t != "variable_declaration" {
        return false;
    }

    // CJS require
    let require_found = require_imports_js(node, source, file_nid, str_path, stem, edges);

    let mut arrow_found = false;
    let mut const_found = false;

    if t == "lexical_declaration" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "variable_declarator"
                    && let Some(value) = child.child_by_field_name("value")
                {
                    if value.kind() == "arrow_function" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let func_name = read_text_owned(name_node, source);
                            let line = child.start_position().row as u32 + 1;
                            let func_nid = make_id(&[stem, &func_name]);
                            add_node(
                                &func_nid,
                                &format!("{func_name}()"),
                                line,
                                str_path,
                                nodes,
                                seen_ids,
                            );
                            add_edge(file_nid, &func_nid, "contains", line, str_path, None, edges);
                            if let Some(body) = value.child_by_field_name("body") {
                                function_bodies.push((func_nid, body));
                            }
                            arrow_found = true;
                        }
                    } else if matches!(
                        value.kind(),
                        "object" | "array" | "as_expression" | "call_expression" | "new_expression"
                    ) && let Some(name_node) = child.child_by_field_name("name")
                    {
                        let const_name = read_text_owned(name_node, source);
                        let line = child.start_position().row as u32 + 1;
                        let const_nid = make_id(&[stem, &const_name]);
                        add_node(&const_nid, &const_name, line, str_path, nodes, seen_ids);
                        add_edge(
                            file_nid, &const_nid, "contains", line, str_path, None, edges,
                        );
                        const_found = true;
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    arrow_found || const_found || require_found
}

// ── CJS require imports ───────────────────────────────────────────────────────

fn find_require_call(value_node: Option<Node<'_>>) -> Option<Node<'_>> {
    let node = value_node?;
    if node.kind() == "call_expression" {
        let fn_node = node.child_by_field_name("function")?;
        if fn_node.kind() == "identifier" {
            return Some(node);
        }
    }
    if node.kind() == "member_expression" {
        let obj = node.child_by_field_name("object")?;
        return find_require_call(Some(obj));
    }
    None
}

#[allow(clippy::too_many_lines)]
fn require_imports_js(
    node: Node<'_>,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    _stem: &str,
    edges: &mut Vec<Edge>,
) -> bool {
    if node.kind() != "lexical_declaration" && node.kind() != "variable_declaration" {
        return false;
    }
    let mut found = false;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return false;
    }
    loop {
        let child = cur.node();
        if child.kind() == "variable_declarator" {
            let value = child.child_by_field_name("value");
            let Some(call) = find_require_call(value) else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            let Some(fn_node) = call.child_by_field_name("function") else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            if read_text(fn_node, source) != "require" {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            }
            let Some(args) = call.child_by_field_name("arguments") else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            let mut raw: Option<String> = None;
            let mut acur = args.walk();
            if acur.goto_first_child() {
                loop {
                    let arg = acur.node();
                    if arg.kind() == "string" {
                        raw = Some(
                            read_text_owned(arg, source)
                                .trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ' ')
                                .to_string(),
                        );
                        break;
                    }
                    if !acur.goto_next_sibling() {
                        break;
                    }
                }
            }
            let Some(raw) = raw else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            let (tgt_nid, resolved_path) = resolve_js_import_target(&raw, str_path);
            let line = node.start_position().row as u32 + 1;
            edges.push(Edge {
                source: file_nid.to_string(),
                target: tgt_nid.clone(),
                relation: "imports_from".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: Some("import".to_string()),
                confidence_score: None,
            });
            found = true;

            // Symbol-level edges
            if let Some(ref rp) = resolved_path {
                let target_stem = file_stem(rp);
                let name_node = child.child_by_field_name("name");
                let mut sym_names: Vec<String> = Vec::new();
                if let Some(nn) = name_node
                    && nn.kind() == "object_pattern"
                {
                    let mut pcur = nn.walk();
                    if pcur.goto_first_child() {
                        loop {
                            let prop = pcur.node();
                            if prop.kind() == "shorthand_property_identifier_pattern" {
                                sym_names.push(read_text_owned(prop, source));
                            } else if prop.kind() == "pair_pattern"
                                && let Some(key) = prop.child_by_field_name("key")
                            {
                                sym_names.push(read_text_owned(key, source));
                            }
                            if !pcur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                // member access: const x = require('./m').y
                if let Some(v) = value
                    && v.kind() == "member_expression"
                    && let Some(prop) = v.child_by_field_name("property")
                {
                    sym_names.push(read_text_owned(prop, source));
                }
                for sym in &sym_names {
                    edges.push(Edge {
                        source: file_nid.to_string(),
                        target: make_id(&[&target_stem, sym]),
                        relation: "imports".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: Some("import".to_string()),
                        confidence_score: None,
                    });
                }
            }
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    found
}

// ── JS import target resolution ───────────────────────────────────────────────

/// Mirrors Python `_resolve_js_import_target`.
/// Returns `(target_nid, Option<resolved_path>)`.
#[must_use]
pub fn resolve_js_import_target(raw: &str, str_path: &str) -> (String, Option<std::path::PathBuf>) {
    if raw.is_empty() {
        return (String::new(), None);
    }
    if raw.starts_with('.') {
        let parent = std::path::Path::new(str_path)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let joined = parent.join(raw);
        let resolved_raw = std::path::PathBuf::from(normalize_path(&joined));
        let resolved = crate::tsconfig::resolve_js_module_path(&resolved_raw);
        return (make_id1(&resolved.to_string_lossy()), Some(resolved));
    }
    let aliases = load_tsconfig_aliases(
        std::path::Path::new(str_path)
            .parent()
            .unwrap_or(std::path::Path::new(".")),
    );
    for (alias_prefix, alias_base) in &aliases {
        if raw == alias_prefix || raw.starts_with(&format!("{alias_prefix}/")) {
            let rest = raw[alias_prefix.len()..].trim_start_matches('/');
            let joined = std::path::Path::new(alias_base).join(rest);
            let resolved_raw = std::path::PathBuf::from(normalize_path(&joined));
            let resolved = crate::tsconfig::resolve_js_module_path(&resolved_raw);
            return (make_id1(&resolved.to_string_lossy()), Some(resolved));
        }
    }
    let module_name = raw.split('/').next_back().unwrap_or(raw);
    if module_name.is_empty() {
        return (String::new(), None);
    }
    (make_id1(module_name), None)
}

/// Normalize path (collapse `.` and `..` components) without requiring the path exists.
fn normalize_path(path: &std::path::Path) -> String {
    let mut components: Vec<&std::ffi::OsStr> = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                }
            }
            std::path::Component::CurDir => {}
            other => components.push(other.as_os_str()),
        }
    }
    std::path::PathBuf::from_iter(components)
        .to_string_lossy()
        .into_owned()
}

// ── Inheritance helpers ───────────────────────────────────────────────────────

fn emit_base_node(
    base: &str,
    _line: u32,
    stem: &str,
    _str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) -> String {
    let nid1 = make_id(&[stem, base]);
    if seen_ids.contains(&nid1) {
        return nid1;
    }
    let nid2 = make_id1(base);
    if !seen_ids.contains(&nid2) {
        nodes.push(GNode {
            id: nid2.clone(),
            label: base.to_string(),
            file_type: "code".to_string(),
            source_file: String::new(),
            source_location: None,
        });
        seen_ids.insert(nid2.clone());
    }
    nid2
}

#[allow(clippy::too_many_arguments)]
fn emit_swift_inheritance(
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "inheritance_specifier" {
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    if matches!(sub.kind(), "user_type" | "type_identifier") {
                        let base = read_text_owned(sub, source);
                        let base_nid = emit_base_node(&base, line, stem, str_path, nodes, seen_ids);
                        add_edge(
                            class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
                    }
                    if !scur.goto_next_sibling() {
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

#[allow(clippy::too_many_arguments)]
fn emit_csharp_inheritance(
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "base_list" {
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    let base = match sub.kind() {
                        "identifier" => Some(read_text_owned(sub, source)),
                        "generic_name" => {
                            if let Some(nc) = sub.child_by_field_name("name") {
                                Some(read_text_owned(nc, source))
                            } else {
                                {
                                    let mut tc = sub.walk();

                                    if tc.goto_first_child() {
                                        Some(tc.node())
                                    } else {
                                        None
                                    }
                                }
                                .map(|first| read_text_owned(first, source))
                            }
                        }
                        _ => None,
                    };
                    if let Some(b) = base
                        && !b.is_empty()
                    {
                        let base_nid = emit_base_node(&b, line, stem, str_path, nodes, seen_ids);
                        add_edge(
                            class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
                    }
                    if !scur.goto_next_sibling() {
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn emit_java_inheritance(
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    node_type: &str,
    line: u32,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
) {
    let emit = |base_name: &str,
                rel: &str,
                nodes: &mut Vec<GNode>,
                edges: &mut Vec<Edge>,
                seen_ids: &mut HashSet<String>| {
        if base_name.is_empty() {
            return;
        }
        let base_nid = emit_base_node(base_name, line, stem, str_path, nodes, seen_ids);
        add_edge(class_nid, &base_nid, rel, line, str_path, None, edges);
    };

    if let Some(sup) = node.child_by_field_name("superclass") {
        let mut cur = sup.walk();
        if cur.goto_first_child() {
            loop {
                let sub = cur.node();
                if sub.kind() == "type_identifier" {
                    emit(
                        &read_text_owned(sub, source),
                        "extends",
                        nodes,
                        edges,
                        seen_ids,
                    );
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    if let Some(ifs) = node.child_by_field_name("interfaces") {
        let mut cur = ifs.walk();
        if cur.goto_first_child() {
            loop {
                let sub = cur.node();
                if sub.kind() == "type_list" {
                    let mut tcur = sub.walk();
                    if tcur.goto_first_child() {
                        loop {
                            let tid = tcur.node();
                            if tid.kind() == "type_identifier" {
                                emit(
                                    &read_text_owned(tid, source),
                                    "implements",
                                    nodes,
                                    edges,
                                    seen_ids,
                                );
                            }
                            if !tcur.goto_next_sibling() {
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
    }

    if node_type == "interface_declaration" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "extends_interfaces" {
                    let mut scur = child.walk();
                    if scur.goto_first_child() {
                        loop {
                            let sub = scur.node();
                            if sub.kind() == "type_list" {
                                let mut tcur = sub.walk();
                                if tcur.goto_first_child() {
                                    loop {
                                        let tid = tcur.node();
                                        if tid.kind() == "type_identifier" {
                                            emit(
                                                &read_text_owned(tid, source),
                                                "extends",
                                                nodes,
                                                edges,
                                                seen_ids,
                                            );
                                        }
                                        if !tcur.goto_next_sibling() {
                                            break;
                                        }
                                    }
                                }
                            }
                            if !scur.goto_next_sibling() {
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
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_cpp_inheritance(
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "base_class_clause" {
            let mut scur = child.walk();
            if scur.goto_first_child() {
                loop {
                    let sub = scur.node();
                    let base = match sub.kind() {
                        "type_identifier" => Some(read_text_owned(sub, source)),
                        "qualified_identifier" | "template_type" => {
                            if let Some(tail) = sub.child_by_field_name("name") {
                                Some(read_text_owned(tail, source))
                            } else {
                                Some(read_text_owned(sub, source))
                            }
                        }
                        _ => None,
                    };
                    if let Some(b) = base
                        && !b.is_empty()
                    {
                        let base_nid = emit_base_node(&b, line, stem, str_path, nodes, seen_ids);
                        add_edge(
                            class_nid, &base_nid, "inherits", line, str_path, None, edges,
                        );
                    }
                    if !scur.goto_next_sibling() {
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

// ── walk_calls ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn walk_calls(
    node: Node<'_>,
    caller_nid: &str,
    source: &[u8],
    config: &LangConfig,
    str_path: &str,
    label_to_nid: &HashMap<String, String>,
    seen_call_pairs: &mut HashSet<(String, String)>,
    seen_dyn_import_pairs: &mut HashSet<(String, String)>,
    edges: &mut Vec<Edge>,
    raw_calls: &mut Vec<RawCall>,
) {
    if config.function_boundary_types.contains(&node.kind()) {
        return;
    }

    if config.call_types.contains(&node.kind()) {
        // JS/TS: detect dynamic import() calls
        if (config.lang_id == LangId::JavaScript
            || config.lang_id == LangId::TypeScript
            || config.lang_id == LangId::TypeScriptX)
            && dynamic_import_js(
                node,
                source,
                caller_nid,
                str_path,
                edges,
                seen_dyn_import_pairs,
            )
        {
            // Still recurse
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    walk_calls(
                        child,
                        caller_nid,
                        source,
                        config,
                        str_path,
                        label_to_nid,
                        seen_call_pairs,
                        seen_dyn_import_pairs,
                        edges,
                        raw_calls,
                    );
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            return;
        }

        let (callee_name, is_member_call) = extract_callee(node, source, config, label_to_nid);

        if let Some(callee) = callee_name
            && !callee.is_empty()
        {
            let tgt_nid = label_to_nid.get(&callee.to_lowercase()).cloned();
            if let Some(tgt) = tgt_nid {
                if tgt != caller_nid {
                    let pair = (caller_nid.to_string(), tgt.clone());
                    if seen_call_pairs.insert(pair) {
                        let line = node.start_position().row as u32 + 1;
                        edges.push(Edge {
                            source: caller_nid.to_string(),
                            target: tgt,
                            relation: "calls".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: Some("call".to_string()),
                            confidence_score: None,
                        });
                    }
                }
            } else {
                raw_calls.push(RawCall {
                    caller_nid: caller_nid.to_string(),
                    callee: callee.clone(),
                    is_member_call,
                    source_file: str_path.to_string(),
                    source_location: format!("L{}", node.start_position().row + 1),
                });
            }
        }
    }

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            walk_calls(
                child,
                caller_nid,
                source,
                config,
                str_path,
                label_to_nid,
                seen_call_pairs,
                seen_dyn_import_pairs,
                edges,
                raw_calls,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn extract_callee(
    node: Node<'_>,
    source: &[u8],
    config: &LangConfig,
    _label_to_nid: &HashMap<String, String>,
) -> (Option<String>, bool) {
    let mut callee_name: Option<String> = None;
    let mut is_member_call = false;

    match config.lang_id {
        LangId::Swift => {
            if let Some(first) = node.child(0) {
                if first.kind() == "simple_identifier" {
                    callee_name = Some(read_text_owned(first, source));
                } else if first.kind() == "navigation_expression" {
                    is_member_call = true;
                    let mut cur = first.walk();
                    if cur.goto_first_child() {
                        loop {
                            let child = cur.node();
                            if child.kind() == "navigation_suffix" {
                                let mut scur = child.walk();
                                if scur.goto_first_child() {
                                    loop {
                                        let sc = scur.node();
                                        if sc.kind() == "simple_identifier" {
                                            callee_name = Some(read_text_owned(sc, source));
                                        }
                                        if !scur.goto_next_sibling() {
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
                }
            }
        }
        LangId::Kotlin => {
            if let Some(first) = node.child(0) {
                if matches!(first.kind(), "simple_identifier" | "identifier") {
                    callee_name = Some(read_text_owned(first, source));
                } else if first.kind() == "navigation_expression" {
                    is_member_call = true;
                    // Reversed scan for last simple_identifier
                    let count = first.child_count();
                    for i in (0..count).rev() {
                        if let Some(c) = first.child(i)
                            && matches!(c.kind(), "simple_identifier" | "identifier")
                        {
                            callee_name = Some(read_text_owned(c, source));
                            break;
                        }
                    }
                }
            }
        }
        LangId::Scala => {
            if let Some(first) = node.child(0) {
                if first.kind() == "identifier" {
                    callee_name = Some(read_text_owned(first, source));
                } else if first.kind() == "field_expression" {
                    is_member_call = true;
                    if let Some(field) = first.child_by_field_name("field") {
                        callee_name = Some(read_text_owned(field, source));
                    } else {
                        let count = first.child_count();
                        for i in (0..count).rev() {
                            if let Some(c) = first.child(i)
                                && c.kind() == "identifier"
                            {
                                callee_name = Some(read_text_owned(c, source));
                                break;
                            }
                        }
                    }
                }
            }
        }
        LangId::CSharp if node.kind() == "invocation_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                callee_name = Some(read_text_owned(name_node, source));
            } else {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        let child = cur.node();
                        if child.is_named() {
                            let raw = read_text_owned(child, source);
                            if raw.contains('.') {
                                callee_name =
                                    Some(raw.split('.').next_back().unwrap_or("").to_string());
                                is_member_call = true;
                            } else {
                                callee_name = Some(raw);
                            }
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        LangId::Php => match node.kind() {
            "function_call_expression" => {
                if let Some(f) = node.child_by_field_name("function") {
                    callee_name = Some(read_text_owned(f, source));
                }
            }
            "scoped_call_expression" => {
                if let Some(scope) = node.child_by_field_name("scope") {
                    callee_name = Some(read_text_owned(scope, source));
                }
            }
            _ => {
                is_member_call = true;
                if let Some(name) = node.child_by_field_name("name") {
                    callee_name = Some(read_text_owned(name, source));
                }
            }
        },
        LangId::Cpp => {
            if !config.call_function_field.is_empty()
                && let Some(func_node) = node.child_by_field_name(config.call_function_field)
            {
                if func_node.kind() == "identifier" {
                    callee_name = Some(read_text_owned(func_node, source));
                } else if matches!(
                    func_node.kind(),
                    "field_expression" | "qualified_identifier"
                ) {
                    is_member_call = true;
                    let name = func_node
                        .child_by_field_name("field")
                        .or_else(|| func_node.child_by_field_name("name"));
                    if let Some(n) = name {
                        callee_name = Some(read_text_owned(n, source));
                    }
                }
            }
        }
        _ => {
            // Generic: use call_function_field
            if !config.call_function_field.is_empty()
                && let Some(func_node) = node.child_by_field_name(config.call_function_field)
            {
                if func_node.kind() == "identifier" {
                    callee_name = Some(read_text_owned(func_node, source));
                } else if config.call_accessor_node_types.contains(&func_node.kind()) {
                    is_member_call = true;
                    if !config.call_accessor_field.is_empty()
                        && let Some(attr) =
                            func_node.child_by_field_name(config.call_accessor_field)
                    {
                        callee_name = Some(read_text_owned(attr, source));
                    }
                } else {
                    callee_name = Some(read_text_owned(func_node, source));
                }
            }
        }
    }

    (callee_name, is_member_call)
}

fn dynamic_import_js(
    node: Node<'_>,
    source: &[u8],
    caller_nid: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
    seen_dyn_pairs: &mut HashSet<(String, String)>,
) -> bool {
    let func_node = node.child_by_field_name("function").or_else(|| {
        let first = node.child(0)?;
        if read_text(first, source) == "import" {
            Some(first)
        } else {
            None
        }
    });
    let Some(func_node) = func_node else {
        return false;
    };
    if read_text(func_node, source) != "import" {
        return false;
    }
    let Some(args) = node.child_by_field_name("arguments") else {
        return true;
    };
    let mut cur = args.walk();
    if !cur.goto_first_child() {
        return true;
    }
    loop {
        let arg = cur.node();
        let raw: Option<String> = if arg.kind() == "template_string" {
            // Skip dynamic template literals with substitutions
            let has_sub = (0..arg.child_count()).any(|i| {
                arg.child(i)
                    .is_some_and(|c| c.kind() == "template_substitution")
            });
            if has_sub {
                None
            } else {
                Some(read_text_owned(arg, source).trim_matches('`').to_string())
            }
        } else if arg.kind() == "string" {
            Some(
                read_text_owned(arg, source)
                    .trim_matches(|c| c == '\'' || c == '"' || c == ' ')
                    .to_string(),
            )
        } else {
            if !cur.goto_next_sibling() {
                break;
            }
            continue;
        };

        let Some(raw) = raw else { break };
        if raw.is_empty() {
            break;
        }

        let (tgt_nid, _) = resolve_js_import_target(&raw, str_path);
        let pair = (caller_nid.to_string(), tgt_nid.clone());
        if seen_dyn_pairs.insert(pair) {
            let line = node.start_position().row as u32 + 1;
            edges.push(Edge {
                source: caller_nid.to_string(),
                target: tgt_nid,
                relation: "imports_from".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
        }
        break;
    }
    true
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Extract nodes and edges from `path` using the given language configuration.
///
/// Mirrors Python `_extract_generic(path, config)`.
#[must_use]
pub fn extract_generic(path: &Path, config: &LangConfig) -> FileResult {
    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return FileResult::error(format!("io error reading {}: {e}", path.display()));
        }
    };

    let mut parser = Parser::new();
    if let Err(e) = parser.set_language(&config.language) {
        return FileResult::error(format!(
            "parser language mismatch for {}: {e}",
            path.display()
        ));
    }

    let Some(tree) = parser.parse(&source, None) else {
        return FileResult::error(format!("tree-sitter parse failed for {}", path.display()));
    };

    let root = tree.root_node();
    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();

    let mut nodes: Vec<GNode> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut function_bodies: Vec<(String, Node<'_>)> = Vec::new();

    let file_nid = make_id1(&str_path);
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    add_node(
        &file_nid,
        &filename,
        1,
        &str_path,
        &mut nodes,
        &mut seen_ids,
    );

    // ── Structural walk ───────────────────────────────────────────────────────
    let mut cur = root.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            walk(
                child,
                None,
                &source,
                config,
                &file_nid,
                &stem,
                &str_path,
                &mut nodes,
                &mut edges,
                &mut seen_ids,
                &mut function_bodies,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    // ── Call-graph pass ───────────────────────────────────────────────────────
    let label_to_nid: HashMap<String, String> = nodes
        .iter()
        .map(|n| {
            let key = n
                .label
                .trim_start_matches('.')
                .trim_end_matches("()")
                .to_lowercase();
            (key, n.id.clone())
        })
        .collect();

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut seen_dyn_import_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();

    for (caller_nid, body_node) in &function_bodies {
        walk_calls(
            *body_node,
            caller_nid,
            &source,
            config,
            &str_path,
            &label_to_nid,
            &mut seen_call_pairs,
            &mut seen_dyn_import_pairs,
            &mut edges,
            &mut raw_calls,
        );
    }

    // ── Clean edges ───────────────────────────────────────────────────────────
    let clean_edges: Vec<Edge> = edges
        .into_iter()
        .filter(|e| {
            seen_ids.contains(&e.source)
                && (seen_ids.contains(&e.target)
                    || matches!(e.relation.as_str(), "imports" | "imports_from"))
        })
        .collect();

    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}
