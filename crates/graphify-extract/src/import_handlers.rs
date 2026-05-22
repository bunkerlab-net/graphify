//! Language-specific import node handlers.
//!
//! Each function matches a Python `_import_<lang>` function from `extract.py`.

// Tree-sitter row numbers represent source line indices; no realistic file has
// 2^32 lines, so the cast from usize to u32 is safe in practice.
#![allow(clippy::cast_possible_truncation)]

use std::path::Path;

use tree_sitter::Node;

use crate::generic::resolve_js_import_target;
use crate::ids::{file_stem, make_id, make_id1};
use crate::types::Edge;

fn read_text_owned(node: Node<'_>, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).into_owned()
}

fn make_edge(
    source: &str,
    target: &str,
    relation: &str,
    context: Option<&str>,
    str_path: &str,
    line: u32,
) -> Edge {
    Edge {
        source: source.to_string(),
        target: target.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: context.map(str::to_string),
        confidence_score: None,
    }
}

// ── Python ────────────────────────────────────────────────────────────────────

pub fn import_python(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let t = node.kind();
    let line = node.start_position().row as u32 + 1;
    if t == "import_statement" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if matches!(child.kind(), "dotted_name" | "aliased_import") {
                    let raw = read_text_owned(child, source);
                    let module_name = raw
                        .split(" as ")
                        .next()
                        .unwrap_or("")
                        .trim()
                        .trim_start_matches('.');
                    let tgt_nid = make_id1(module_name);
                    edges.push(make_edge(
                        file_nid,
                        &tgt_nid,
                        "imports",
                        Some("import"),
                        str_path,
                        line,
                    ));
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    } else if t == "import_from_statement"
        && let Some(module_node) = node.child_by_field_name("module_name")
    {
        let raw = read_text_owned(module_node, source);
        let tgt_nid = if raw.starts_with('.') {
            let dots = raw.len() - raw.trim_start_matches('.').len();
            let module_name = raw.trim_start_matches('.');
            let mut base = Path::new(str_path)
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf();
            for _ in 0..(dots - 1) {
                base = base.parent().unwrap_or(Path::new(".")).to_path_buf();
            }
            let rel = if module_name.is_empty() {
                "__init__.py".to_string()
            } else {
                format!("{}.py", module_name.replace('.', "/"))
            };
            make_id1(&base.join(rel).to_string_lossy())
        } else {
            make_id1(&raw)
        };
        edges.push(make_edge(
            file_nid,
            &tgt_nid,
            "imports_from",
            Some("import"),
            str_path,
            line,
        ));
    }
}

// ── JavaScript / TypeScript ───────────────────────────────────────────────────

pub fn import_js(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let mut resolved_path: Option<std::path::PathBuf> = None;

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "string" {
                let raw = read_text_owned(child, source)
                    .trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ' ')
                    .to_string();
                if raw.is_empty() {
                    break;
                }
                let (tgt_nid, rp) = resolve_js_import_target(&raw, str_path);
                if tgt_nid.is_empty() {
                    break;
                }
                resolved_path = rp;
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports_from",
                    Some("import"),
                    str_path,
                    line,
                ));
                break;
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    // Named imports: `import { Foo, Bar } from './bar'`
    if let Some(ref rp) = resolved_path {
        let target_stem = file_stem(rp);
        let mut cur2 = node.walk();
        if cur2.goto_first_child() {
            loop {
                let child = cur2.node();
                if child.kind() == "import_clause" {
                    let mut scur = child.walk();
                    if scur.goto_first_child() {
                        loop {
                            let sub = scur.node();
                            if sub.kind() == "named_imports" {
                                let mut ncur = sub.walk();
                                if ncur.goto_first_child() {
                                    loop {
                                        let spec = ncur.node();
                                        if spec.kind() == "import_specifier"
                                            && let Some(name_node) =
                                                spec.child_by_field_name("name")
                                        {
                                            let sym = read_text_owned(name_node, source);
                                            edges.push(make_edge(
                                                file_nid,
                                                &make_id(&[&target_stem, &sym]),
                                                "imports",
                                                Some("import"),
                                                str_path,
                                                line,
                                            ));
                                        }
                                        if !ncur.goto_next_sibling() {
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
                if !cur2.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

// ── Java ──────────────────────────────────────────────────────────────────────

pub fn import_java(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if matches!(child.kind(), "scoped_identifier" | "identifier") {
            let path_str = walk_scoped_java(child, source);
            let parts: Vec<&str> = path_str.split('.').collect();
            let module_name = parts
                .last()
                .map_or("", |s| s.trim_end_matches('*').trim_matches('.'))
                .to_string();
            let module_name = if module_name.is_empty() && parts.len() > 1 {
                parts[parts.len() - 2].to_string()
            } else {
                module_name
            };
            if !module_name.is_empty() {
                let tgt_nid = make_id1(&module_name);
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports",
                    Some("import"),
                    str_path,
                    line,
                ));
            }
            break;
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

fn walk_scoped_java(node: Node<'_>, source: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = node;
    loop {
        if cur.kind() == "scoped_identifier" {
            if let Some(name_node) = cur.child_by_field_name("name") {
                parts.push(read_text_owned(name_node, source));
            }
            if let Some(scope) = cur.child_by_field_name("scope") {
                cur = scope;
            } else {
                break;
            }
        } else if cur.kind() == "identifier" {
            parts.push(read_text_owned(cur, source));
            break;
        } else {
            break;
        }
    }
    parts.reverse();
    parts.join(".")
}

// ── C/C++ ─────────────────────────────────────────────────────────────────────

pub fn import_c(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if matches!(
            child.kind(),
            "string_literal" | "system_lib_string" | "string"
        ) {
            let raw = read_text_owned(child, source)
                .trim_matches(|c: char| matches!(c, '"' | '<' | '>' | ' '))
                .to_string();
            // Quoted includes: try to resolve to file
            if child.kind() != "system_lib_string"
                && let Some(resolved) = resolve_c_include_path(&raw, str_path)
            {
                let tgt_nid = make_id1(&resolved.to_string_lossy());
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports",
                    Some("import"),
                    str_path,
                    line,
                ));
                break;
            }
            let module_name = raw
                .split('/')
                .next_back()
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("");
            if !module_name.is_empty() {
                let tgt_nid = make_id1(module_name);
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports",
                    Some("import"),
                    str_path,
                    line,
                ));
            }
            break;
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

fn resolve_c_include_path(raw: &str, str_path: &str) -> Option<std::path::PathBuf> {
    if raw.is_empty() {
        return None;
    }
    let candidate = Path::new(str_path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(raw);
    let canonical = candidate.canonicalize().ok()?;
    if canonical.is_file() {
        Some(canonical)
    } else {
        None
    }
}

// ── C# ────────────────────────────────────────────────────────────────────────

pub fn import_csharp(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if matches!(
            child.kind(),
            "qualified_name" | "identifier" | "name_equals"
        ) {
            let raw = read_text_owned(child, source);
            let module_name = raw.split('.').next_back().unwrap_or("").trim().to_string();
            if !module_name.is_empty() {
                let tgt_nid = make_id1(&module_name);
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports",
                    Some("import"),
                    str_path,
                    line,
                ));
            }
            break;
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

// ── Kotlin ───────────────────────────────────────────────────────────────────

pub fn import_kotlin(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    if let Some(path_node) = node.child_by_field_name("path") {
        let raw = read_text_owned(path_node, source);
        let module_name = raw.split('.').next_back().unwrap_or("").trim().to_string();
        if !module_name.is_empty() {
            let tgt_nid = make_id1(&module_name);
            edges.push(make_edge(
                file_nid,
                &tgt_nid,
                "imports",
                Some("import"),
                str_path,
                line,
            ));
        }
        return;
    }
    // Fallback: identifier child
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "identifier" {
                let raw = read_text_owned(child, source);
                let tgt_nid = make_id1(&raw);
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports",
                    Some("import"),
                    str_path,
                    line,
                ));
                break;
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

// ── Scala ─────────────────────────────────────────────────────────────────────

pub fn import_scala(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if matches!(child.kind(), "stable_id" | "identifier") {
            let raw = read_text_owned(child, source);
            let module_name = raw
                .split('.')
                .next_back()
                .unwrap_or("")
                .trim_matches(|c: char| matches!(c, '{' | '}' | ' '))
                .trim()
                .to_string();
            if !module_name.is_empty() && module_name != "_" {
                let tgt_nid = make_id1(&module_name);
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports",
                    Some("import"),
                    str_path,
                    line,
                ));
            }
            break;
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

// ── PHP ───────────────────────────────────────────────────────────────────────

pub fn import_php(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if matches!(child.kind(), "qualified_name" | "name" | "identifier") {
            let raw = read_text_owned(child, source);
            let module_name = raw.split('\\').next_back().unwrap_or("").trim().to_string();
            if !module_name.is_empty() {
                let tgt_nid = make_id1(&module_name);
                edges.push(make_edge(
                    file_nid,
                    &tgt_nid,
                    "imports",
                    Some("import"),
                    str_path,
                    line,
                ));
            }
            break;
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

// ── Lua ───────────────────────────────────────────────────────────────────────

pub fn import_lua(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let text = String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).into_owned();
    let line = node.start_position().row as u32 + 1;
    // regex: require\s*[('"\s]*['"]?([^'")\s]+)
    if let Some(cap) = find_require_module(&text) {
        let module_name = cap.split('.').next_back().unwrap_or("").to_string();
        if !module_name.is_empty() {
            edges.push(Edge {
                source: file_nid.to_string(),
                target: module_name.clone(),
                relation: "imports".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: Some("import".to_string()),
                confidence_score: Some(1.0),
            });
        }
    }
}

fn find_require_module(text: &str) -> Option<String> {
    // require\s*[\('"]\s*['"]?([^'")\s]+)
    let start = text.find("require")?;
    let rest = &text[start + "require".len()..];
    let inner = rest.trim_start_matches([' ', '(', '\'', '"']);
    // Find end of module name
    let end = inner
        .find(['\'', '"', ')', ' ', '\t', '\n'])
        .unwrap_or(inner.len());
    let module = &inner[..end];
    if module.is_empty() {
        None
    } else {
        Some(module.to_string())
    }
}

// ── Swift ─────────────────────────────────────────────────────────────────────

pub fn import_swift(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() == "identifier" {
            let raw = read_text_owned(child, source);
            let tgt_nid = make_id1(&raw);
            edges.push(make_edge(
                file_nid,
                &tgt_nid,
                "imports",
                Some("import"),
                str_path,
                line,
            ));
            break;
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}
