//! Public extractor functions — one per language (or group of related languages).
//!
//! Each function mirrors a Python `extract_<lang>` function from `extract.py`.

pub mod bash;
pub mod blade;
pub mod dart;
pub mod dotnet;
pub mod elixir;
pub mod fortran;
pub mod go;
pub mod json_lang;
pub mod julia;
pub mod markdown;
pub mod mcp;
pub mod multi;
pub mod objc;
pub mod pascal;
pub mod powershell;
pub mod rust_lang;
pub mod sql;
pub mod svelte;
pub mod verilog;
pub mod zig;

use std::path::Path;

use crate::generic::extract_generic;
use crate::lang_configs;
use crate::types::FileResult;

pub use multi::extract;

const RATIONALE_PREFIXES: &[&str] = &[
    "# NOTE:",
    "# IMPORTANT:",
    "# HACK:",
    "# WHY:",
    "# RATIONALE:",
    "# TODO:",
    "# FIXME:",
];

/// Size cap for project XML files (`.csproj` / `.fsproj` / `.vbproj` / `.lpk`).
/// Real files are well under 2 MiB; anything larger is malformed or hostile.
/// Mirrors `_PROJECT_XML_MAX_BYTES` in `graphify-py`.
pub(crate) const PROJECT_XML_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Reject project XML that declares a DTD or entities.
///
/// Defense in depth against billion-laughs style entity-expansion `DoS`.
/// Legitimate `MSBuild` and Lazarus package files never contain a `<!DOCTYPE`
/// or `<!ENTITY` declaration, so this is a zero-false-positive screen.
/// Mirrors `_project_xml_is_safe` in `graphify-py`.
#[must_use]
pub(crate) fn project_xml_is_safe(src: &[u8]) -> bool {
    // Scan the raw bytes with an ASCII case-insensitive window match rather
    // than allocating a lowercase copy of the whole (up to 2 MiB) file.
    fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
    }
    !contains_ci(src, b"<!doctype") && !contains_ci(src, b"<!entity")
}

// ── Python ────────────────────────────────────────────────────────────────────

/// Extract classes, functions, and imports from a `.py` file.
#[must_use]
pub fn extract_python(path: &Path) -> FileResult {
    let mut result = extract_generic(path, &lang_configs::PYTHON);
    if result.error.is_none() {
        extract_python_rationale(path, &mut result);
    }
    result
}

// ── JavaScript / TypeScript ───────────────────────────────────────────────────

/// Extract classes, functions, arrow functions, and imports from `.js`/`.ts`/`.tsx` files.
#[must_use]
pub fn extract_js(path: &Path) -> FileResult {
    let config = match path.extension().and_then(|e| e.to_str()) {
        Some("tsx") => &*lang_configs::TYPESCRIPT_TSX,
        Some("ts") => &*lang_configs::TYPESCRIPT,
        _ => &*lang_configs::JAVASCRIPT,
    };
    extract_generic(path, config)
}

// ── Java ──────────────────────────────────────────────────────────────────────

/// Extract classes, interfaces, methods, constructors, and imports from a `.java` file.
#[must_use]
pub fn extract_java(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::JAVA)
}

// ── Groovy ────────────────────────────────────────────────────────────────────

/// Extract classes, methods, constructors, and imports from a `.groovy`/`.gradle` file.
/// Falls back to regex-based Spock extractor when needed.
#[must_use]
pub fn extract_groovy(path: &Path) -> FileResult {
    let result = extract_generic(path, &lang_configs::GROOVY);
    if is_spock_file(path) {
        extract_spock_fallback(path, result)
    } else {
        result
    }
}

/// Return `true` if the Groovy file contains Spock-style `def "feature"()` test methods.
///
/// Spock test methods use quoted string names that the generic tree-sitter extractor misses;
/// this heuristic triggers the regex fallback when any line starts with `def "` or `def '`.
fn is_spock_file(path: &Path) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };
    // Check for `def "feature"()` patterns
    src.lines().any(|l| {
        let t = l.trim();
        t.starts_with("def \"") || t.starts_with("def '")
    })
}

/// Extract class and method nodes from a Spock test file using regex scanning.
///
/// The generic tree-sitter pass already ran (`ts_result`) but cannot handle Spock's quoted
/// method names. This function discards the tree-sitter node/method edges, keeps the file
/// node and import edges, then re-scans line-by-line with three regexes:
/// `class`, `def "feature"()`, and `def plainMethod()`. Mirrors Python `_extract_spock_fallback`.
#[allow(
    clippy::too_many_lines,
    clippy::expect_used,
    clippy::cast_possible_truncation
)]
// ↑ literal regex patterns; function is a direct port; row→u32 is safe
fn extract_spock_fallback(path: &Path, ts_result: FileResult) -> FileResult {
    use crate::ids::{file_stem, make_id, make_id1};
    use crate::types::{Edge, Node};
    use std::collections::HashSet;

    let Ok(source) = std::fs::read_to_string(path) else {
        return ts_result;
    };
    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);

    // Keep file node + import edges from tree-sitter pass
    let file_node = ts_result
        .nodes
        .iter()
        .find(|n| {
            path.file_name()
                .is_some_and(|f| f.to_string_lossy() == n.label)
        })
        .cloned();
    let mut nodes: Vec<Node> = file_node.into_iter().collect();
    let mut edges: Vec<Edge> = ts_result
        .edges
        .into_iter()
        .filter(|e| e.context.as_deref() == Some("import"))
        .collect();
    let mut seen_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

    let file_nid = make_id1(&str_path);
    if !seen_ids.contains(&file_nid) {
        nodes.push(Node {
            id: file_nid.clone(),
            label: path
                .file_name()
                .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
            file_type: "code".to_string(),
            source_file: str_path.clone(),
            source_location: Some("L1".to_string()),
            metadata: None,
        });
        seen_ids.insert(file_nid.clone());
    }

    let class_re =
        regex::Regex::new(r"^\s*(?:[\w@]+\s+)*class\s+(\w+)").expect("static spock class regex");
    let feature_re = regex::Regex::new(r#"^\s*def\s+(?:"([^"]+)"|'([^']+)')\s*\("#)
        .expect("static spock feature regex");
    let plain_method_re =
        regex::Regex::new(r"^\s*def\s+(\w+)\s*\(").expect("static spock method regex");
    let kws: std::collections::HashSet<&str> = ["if", "while", "for", "switch", "catch"]
        .iter()
        .copied()
        .collect();

    let mut current_class_nid: Option<String> = None;

    for (lineno, line) in source.lines().enumerate() {
        let lineno = lineno + 1;
        if let Some(cap) = class_re.captures(line) {
            let class_name = cap.get(1).map_or("", |m| m.as_str());
            let class_nid = make_id(&[&stem, class_name]);
            if !seen_ids.contains(&class_nid) {
                seen_ids.insert(class_nid.clone());
                nodes.push(Node {
                    id: class_nid.clone(),
                    label: class_name.to_string(),
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{lineno}")),
                    metadata: None,
                });
            }
            edges.push(Edge {
                source: file_nid.clone(),
                target: class_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{lineno}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            current_class_nid = Some(class_nid);
            continue;
        }

        let Some(ref class_nid) = current_class_nid else {
            continue;
        };

        if let Some(cap) = feature_re.captures(line) {
            let method_name = cap.get(1).or_else(|| cap.get(2)).map_or("", |m| m.as_str());
            let method_label = format!("\"{method_name}\"");
            let method_nid = make_id(&[class_nid, method_name]);
            if !seen_ids.contains(&method_nid) {
                seen_ids.insert(method_nid.clone());
                nodes.push(Node {
                    id: method_nid.clone(),
                    label: method_label,
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{lineno}")),
                    metadata: None,
                });
            }
            edges.push(Edge {
                source: class_nid.clone(),
                target: method_nid,
                relation: "method".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{lineno}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            continue;
        }

        if let Some(cap) = plain_method_re.captures(line) {
            let method_name = cap.get(1).map_or("", |m| m.as_str());
            if !kws.contains(method_name) {
                let method_label = format!(".{method_name}()");
                let method_nid = make_id(&[class_nid, method_name]);
                if !seen_ids.contains(&method_nid) {
                    seen_ids.insert(method_nid.clone());
                    nodes.push(Node {
                        id: method_nid.clone(),
                        label: method_label,
                        file_type: "code".to_string(),
                        source_file: str_path.clone(),
                        source_location: Some(format!("L{lineno}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    source: class_nid.clone(),
                    target: method_nid,
                    relation: "method".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{lineno}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
            }
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: Vec::new(),
        error: None,
    }
}

// ── C ─────────────────────────────────────────────────────────────────────────

/// Extract functions and includes from a `.c`/`.h` file.
#[must_use]
pub fn extract_c(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::C)
}

// ── C++ ───────────────────────────────────────────────────────────────────────

/// Extract functions, classes, and includes from a `.cpp`/`.cc`/`.cxx`/`.hpp` file.
#[must_use]
pub fn extract_cpp(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::CPP)
}

// ── Ruby ──────────────────────────────────────────────────────────────────────

/// Extract classes, methods, singleton methods, and calls from a `.rb` file.
#[must_use]
pub fn extract_ruby(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::RUBY)
}

// ── C# ────────────────────────────────────────────────────────────────────────

/// Extract classes, interfaces, methods, namespaces, and usings from a `.cs` file.
#[must_use]
pub fn extract_csharp(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::CSHARP)
}

// ── Kotlin ────────────────────────────────────────────────────────────────────

/// Extract classes, objects, functions, and imports from a `.kt`/`.kts` file.
#[must_use]
pub fn extract_kotlin(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::KOTLIN)
}

// ── Scala ─────────────────────────────────────────────────────────────────────

/// Extract classes, objects, functions, and imports from a `.scala` file.
#[must_use]
pub fn extract_scala(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::SCALA)
}

// ── PHP ───────────────────────────────────────────────────────────────────────

/// Extract classes, functions, methods, namespace uses, and calls from a `.php` file.
#[must_use]
pub fn extract_php(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::PHP)
}

// ── Lua ───────────────────────────────────────────────────────────────────────

/// Extract functions, methods, and `require()` imports from a `.lua` file.
#[must_use]
pub fn extract_lua(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::LUA)
}

// ── Swift ─────────────────────────────────────────────────────────────────────

/// Extract classes, structs, protocols, functions, imports, and calls from a `.swift` file.
#[must_use]
pub fn extract_swift(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::SWIFT)
}

// ── Go ────────────────────────────────────────────────────────────────────────
pub use go::extract_go;

// ── Rust ──────────────────────────────────────────────────────────────────────
pub use rust_lang::extract_rust;

// ── Zig ───────────────────────────────────────────────────────────────────────
pub use zig::extract_zig;

// ── PowerShell ────────────────────────────────────────────────────────────────
pub use powershell::extract_powershell;

// ── Elixir ────────────────────────────────────────────────────────────────────
pub use elixir::extract_elixir;

// ── Julia ─────────────────────────────────────────────────────────────────────
pub use julia::extract_julia;

// ── Fortran ───────────────────────────────────────────────────────────────────
pub use fortran::extract_fortran;

// ── ObjC ──────────────────────────────────────────────────────────────────────
pub use objc::extract_objc;

// ── Bash ──────────────────────────────────────────────────────────────────────
pub use bash::extract_bash;

// ── JSON ──────────────────────────────────────────────────────────────────────
pub use json_lang::extract_json;

// ── Verilog ───────────────────────────────────────────────────────────────────
pub use verilog::extract_verilog;

// ── SQL ───────────────────────────────────────────────────────────────────────
pub use sql::extract_sql;

// ── Markdown ──────────────────────────────────────────────────────────────────
pub use markdown::extract_markdown;

// ── Pascal ────────────────────────────────────────────────────────────────────
pub use pascal::{
    extract_delphi_form, extract_lazarus_form, extract_lazarus_package, extract_pascal,
};

// ── Svelte / Astro ────────────────────────────────────────────────────────────
pub use svelte::{extract_astro, extract_svelte};

// ── Dart ──────────────────────────────────────────────────────────────────────
pub use dart::extract_dart;

// ── MCP config (.mcp.json / claude_desktop_config.json / ...) ─────────────────
pub use mcp::{extract_mcp_config, is_mcp_config_path};

// ── Blade ─────────────────────────────────────────────────────────────────────
pub use blade::extract_blade;

// ── .NET (.sln / .csproj / .razor) ────────────────────────────────────────────
pub use dotnet::{extract_csproj, extract_razor, extract_sln};

// ── Python rationale extraction ───────────────────────────────────────────────

/// Augment a Python extraction result with rationale nodes sourced from docstrings and comments.
///
/// Walks the file's AST for module, class, and function docstrings (> 20 chars) and scans
/// source lines for `RATIONALE_PREFIXES` comments. Each rationale becomes a node of
/// `file_type = "rationale"` connected via a `rationale_for` edge to the containing entity.
/// Auto-generated files (migrations, protobuf, Alembic) are silently skipped.
/// Mirrors Python `_extract_rationale`.
fn extract_python_rationale(path: &Path, result: &mut FileResult) {
    use crate::ids::{file_stem, make_id, make_id1};
    use crate::types::{Edge, Node};
    use std::collections::HashSet;
    use tree_sitter::Parser;

    let Ok(source) = std::fs::read(path) else {
        return;
    };

    // Skip auto-generated files
    if is_autogenerated_python(&source) {
        return;
    }

    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .is_err()
    {
        return;
    }
    let Some(tree) = parser.parse(&source, None) else {
        return;
    };

    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    let mut seen_ids: HashSet<String> = result.nodes.iter().map(|n| n.id.clone()).collect();

    let add_rationale = |text: &str,
                         line: u32,
                         parent_nid: &str,
                         seen: &mut HashSet<String>,
                         nodes: &mut Vec<Node>,
                         edges: &mut Vec<Edge>| {
        let label: String = text
            .chars()
            .take(80)
            .collect::<String>()
            .replace("\r\n", " ")
            .replace(['\r', '\n'], " ")
            .trim()
            .to_string();
        let rid = make_id(&[&stem, "rationale", &line.to_string()]);
        if seen.insert(rid.clone()) {
            nodes.push(Node {
                id: rid.clone(),
                label,
                file_type: "rationale".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{line}")),
                metadata: None,
            });
        }
        edges.push(Edge {
            source: rid,
            target: parent_nid.to_string(),
            relation: "rationale_for".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    };

    // Module-level docstring
    let root = tree.root_node();
    if let Some((doc, line)) = get_docstring(root, &source) {
        add_rationale(
            &doc,
            line,
            &file_nid,
            &mut seen_ids,
            &mut result.nodes,
            &mut result.edges,
        );
    }

    // Walk class/function docstrings
    {
        let mut doc_ctx = DocstringWalkCtx {
            stem: &stem,
            file_nid: &file_nid,
            seen_ids: &mut seen_ids,
            nodes: &mut result.nodes,
            edges: &mut result.edges,
            add_rationale: &add_rationale,
        };
        walk_docstrings(&mut doc_ctx, root, &file_nid, &source);
    }

    // Rationale comments
    let source_text = String::from_utf8_lossy(&source).into_owned();
    for (lineno, line_text) in source_text.lines().enumerate() {
        let stripped = line_text.trim();
        if RATIONALE_PREFIXES.iter().any(|p| stripped.starts_with(p)) {
            add_rationale(
                stripped,
                u32::try_from(lineno).unwrap_or(u32::MAX).saturating_add(1),
                &file_nid,
                &mut seen_ids,
                &mut result.nodes,
                &mut result.edges,
            );
        }
    }
}

/// Return `true` when the Python source is auto-generated and should not have rationale extracted.
///
/// Checks the first 2048 bytes for `DO NOT EDIT`, `@generated`, or protobuf markers, and also
/// detects Alembic/Flask-Migrate migration files and Django migration classes. Mirrors Python
/// `_is_autogenerated`.
fn is_autogenerated_python(source: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&source[..source.len().min(2048)]).into_owned();
    if head.contains("DO NOT EDIT")
        || head.contains("@generated")
        || head.contains("Generated by the protocol buffer")
    {
        return true;
    }
    // Alembic / Flask-Migrate
    if head.contains("def upgrade(")
        && head.contains("down_revision")
        && head.lines().any(|l| {
            let t = l.trim();
            t.starts_with("revision") && (t.contains(':') || t.contains('='))
        })
    {
        return true;
    }
    // Django migrations
    if head.contains("class Migration(migrations.Migration)") && head.contains("operations") {
        return true;
    }
    false
}

/// Extract the first triple-quoted docstring from a Python AST node's first child.
///
/// Looks for an `expression_statement` as the first child containing a `string` or
/// `concatenated_string` node; returns `(cleaned_text, line_number)` when the cleaned text
/// exceeds 20 characters (too-short strings are likely not real docstrings).
fn get_docstring(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<(String, u32)> {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return None;
    }
    let child = cur.node();
    if child.kind() == "expression_statement" {
        let mut ecur = child.walk();
        if ecur.goto_first_child() {
            loop {
                let sub = ecur.node();
                if matches!(sub.kind(), "string" | "concatenated_string") {
                    let text = String::from_utf8_lossy(&source[sub.start_byte()..sub.end_byte()])
                        .into_owned();
                    let clean = text
                        .trim_matches('"')
                        .trim_matches('\'')
                        .trim_start_matches("\"\"\"")
                        .trim_end_matches("\"\"\"")
                        .trim_start_matches("'''")
                        .trim_end_matches("'''")
                        .trim()
                        .to_string();
                    if clean.len() > 20 {
                        let row = child.start_position().row;
                        return Some((
                            clean,
                            u32::try_from(row).unwrap_or(u32::MAX).saturating_add(1),
                        ));
                    }
                }
                if !ecur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    None
}

/// Recursively walk a Python AST node extracting docstrings from class and function bodies.
///
/// For `class_definition` nodes, extracts the class body docstring and recurses into methods.
/// For `function_definition` nodes, extracts the function body docstring and stops recursing.
/// All other nodes are traversed without emitting rationale. Called by `extract_python_rationale`.
/// Shared state threaded through every [`walk_docstrings`] recursion.
struct DocstringWalkCtx<'a, F>
where
    F: Fn(
        &str,
        u32,
        &str,
        &mut std::collections::HashSet<String>,
        &mut Vec<crate::types::Node>,
        &mut Vec<crate::types::Edge>,
    ),
{
    stem: &'a str,
    file_nid: &'a str,
    seen_ids: &'a mut std::collections::HashSet<String>,
    nodes: &'a mut Vec<crate::types::Node>,
    edges: &'a mut Vec<crate::types::Edge>,
    add_rationale: &'a F,
}

fn walk_docstrings<F>(
    ctx: &mut DocstringWalkCtx<'_, F>,
    node: tree_sitter::Node<'_>,
    parent_nid: &str,
    source: &[u8],
) where
    F: Fn(
        &str,
        u32,
        &str,
        &mut std::collections::HashSet<String>,
        &mut Vec<crate::types::Node>,
        &mut Vec<crate::types::Edge>,
    ),
{
    use crate::ids::make_id;
    let t = node.kind();
    if t == "class_definition" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let class_name =
                String::from_utf8_lossy(&source[name_node.start_byte()..name_node.end_byte()])
                    .into_owned();
            let nid = make_id(&[ctx.stem, &class_name]);
            if let Some(body) = node.child_by_field_name("body") {
                if let Some((doc, line)) = get_docstring(body, source) {
                    (ctx.add_rationale)(&doc, line, &nid, ctx.seen_ids, ctx.nodes, ctx.edges);
                }
                let mut cur = body.walk();
                if cur.goto_first_child() {
                    loop {
                        let child = cur.node();
                        walk_docstrings(ctx, child, &nid, source);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        return;
    }
    if t == "function_definition" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let func_name =
                String::from_utf8_lossy(&source[name_node.start_byte()..name_node.end_byte()])
                    .into_owned();
            let nid = if parent_nid == ctx.file_nid {
                make_id(&[ctx.stem, &func_name])
            } else {
                make_id(&[parent_nid, &func_name])
            };
            if let Some(body) = node.child_by_field_name("body")
                && let Some((doc, line)) = get_docstring(body, source)
            {
                (ctx.add_rationale)(&doc, line, &nid, ctx.seen_ids, ctx.nodes, ctx.edges);
            }
        }
        return;
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            walk_docstrings(ctx, child, parent_nid, source);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
