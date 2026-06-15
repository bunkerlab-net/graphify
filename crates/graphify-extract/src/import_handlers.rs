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

/// Return the source bytes covered by `node` as an owned `String` (lossy UTF-8).
fn read_text_owned(node: Node<'_>, source: &[u8]) -> String {
    String::from_utf8_lossy(&source[node.start_byte()..node.end_byte()]).into_owned()
}

/// Construct an `Edge` with standard extraction defaults (confidence `"EXTRACTED"`, weight `1.0`).
pub(crate) fn make_edge(
    source: &str,
    target: &str,
    relation: &str,
    context: Option<&str>,
    str_path: &str,
    line: u32,
) -> Edge {
    Edge {
        external: false,
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

/// Emit `imports` / `imports_from` edges for a Python `import_statement` or `import_from_statement` node.
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

        // #1146: `from pkg import submod` — when the module resolves to a
        // package on disk and an imported name is itself a submodule file,
        // emit a file-level `imports_from`/`submodule_import` edge to that
        // submodule so package-form imports don't leave the submodule as a
        // disconnected island.
        if let Some(pkg_dir) = resolve_python_package_dir(&raw, str_path) {
            for imported_name in python_imported_names(source, node) {
                let sub_py = pkg_dir.join(format!("{imported_name}.py"));
                let sub_pkg = pkg_dir.join(&imported_name).join("__init__.py");
                let submodule = if sub_py.is_file() {
                    Some(sub_py)
                } else if sub_pkg.is_file() {
                    Some(sub_pkg)
                } else {
                    None
                };
                if let Some(submodule) = submodule {
                    edges.push(make_edge(
                        file_nid,
                        &make_id1(&submodule.to_string_lossy()),
                        "imports_from",
                        Some("submodule_import"),
                        str_path,
                        line,
                    ));
                }
            }
        }
    }
}

/// How many ancestor directories to probe when locating a package root for an
/// absolute `from pkg import …` (#1146). The Rust per-file extractor has no
/// project-root parameter, so it discovers the root by walking up from the
/// importing file — mirroring the `require()` resolver. Six levels covers
/// realistic source layouts without scanning unbounded ancestry.
const PYTHON_PKG_PROBE_LEVELS: usize = 6;

/// Resolve a Python `from <module> import …` module name to the package
/// directory on disk (the directory containing its `__init__.py`), or `None`
/// when the module is not a package — i.e. it resolves to a plain `.py` file or
/// cannot be found.
///
/// Mirrors the package branch of `_resolve_python_module_path`. Relative
/// imports resolve against the importing file's directory; absolute imports
/// walk up from it to discover the project root.
fn resolve_python_package_dir(raw: &str, str_path: &str) -> Option<std::path::PathBuf> {
    let file_dir = Path::new(str_path).parent().unwrap_or(Path::new("."));
    if raw.starts_with('.') {
        let dots = raw.len() - raw.trim_start_matches('.').len();
        let module_name = raw.trim_start_matches('.');
        let mut base = file_dir.to_path_buf();
        for _ in 0..(dots - 1) {
            base = base.parent().unwrap_or(Path::new(".")).to_path_buf();
        }
        let candidate = if module_name.is_empty() {
            base
        } else {
            base.join(module_name.replace('.', "/"))
        };
        return package_dir_if_init(&candidate);
    }
    // Absolute import: probe upward for an ancestor that hosts the package.
    let rel = raw.replace('.', "/");
    let mut probe = file_dir.to_path_buf();
    for _ in 0..PYTHON_PKG_PROBE_LEVELS {
        if let Some(dir) = package_dir_if_init(&probe.join(&rel)) {
            return Some(dir);
        }
        match probe.parent() {
            Some(p) if p != probe => probe = p.to_path_buf(),
            _ => break,
        }
    }
    None
}

/// Return `candidate` when it is a directory containing an `__init__.py`.
fn package_dir_if_init(candidate: &Path) -> Option<std::path::PathBuf> {
    if candidate.is_dir() && candidate.join("__init__.py").is_file() {
        Some(candidate.to_path_buf())
    } else {
        None
    }
}

/// Collect the imported leaf names from a Python `import_from_statement`,
/// mirroring `_python_imported_names`. Returns the full dotted name for plain
/// specifiers and the original (non-alias) name for `x as y` specifiers — the
/// alias is irrelevant to on-disk submodule resolution.
fn python_imported_names(source: &[u8], node: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();
    let mut past_import = false;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return names;
    }
    loop {
        let child = cur.node();
        match child.kind() {
            "import" => past_import = true,
            "dotted_name" if past_import => names.push(read_text_owned(child, source)),
            "aliased_import" if past_import => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    names.push(read_text_owned(name_node, source));
                }
            }
            _ => {}
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    names
}

// ── JavaScript / TypeScript ───────────────────────────────────────────────────

/// Emit `imports_from` / `imports` / `re_exports` edges for a JS/TS
/// `import_statement` or `export_statement` node.
///
/// Re-export shape: `export { Foo } from './bar'` matches the
/// `export_statement` path and emits `re_exports` edges with
/// `context="re-export"` for each specifier, plus a single `imports_from`
/// edge to the source module. A pure `export { foo }` with no `from`
/// clause is skipped — local re-binding of an already-declared symbol
/// produces no inter-file edge.
///
/// Mirrors `_import_js` in graphify-py `extract.py`.
pub fn import_js(
    source: &[u8],
    node: Node<'_>,
    file_nid: &str,
    _stem: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    let line = node.start_position().row as u32 + 1;
    let is_export = node.kind() == "export_statement";

    // Find the source-module `string` child. For an `export_statement`,
    // its absence means a pure local re-bind (`export { foo }`) — skip.
    let mut resolved_path: Option<std::path::PathBuf> = None;
    let mut found_string = false;
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() == "string" {
                found_string = true;
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
    if is_export && !found_string {
        return;
    }

    let Some(ref rp) = resolved_path else { return };
    let target_stem = file_stem(rp);

    // Walk either `import_clause` (for imports) or `export_clause` (for
    // re-exports). The clause names differ but the named-specifier shape
    // is the same.
    let (clause_kind, relation, context) = if is_export {
        ("export_clause", "re_exports", "re-export")
    } else {
        ("import_clause", "imports", "import")
    };

    let mut cur2 = node.walk();
    if cur2.goto_first_child() {
        loop {
            let child = cur2.node();
            if child.kind() == clause_kind {
                walk_specifiers(
                    source,
                    child,
                    file_nid,
                    &target_stem,
                    str_path,
                    relation,
                    context,
                    line,
                    edges,
                );
            }
            if !cur2.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Walk a `named_imports` (for `import_clause`) or the `export_specifier`
/// children directly (for `export_clause`), emitting one edge per specifier.
#[allow(clippy::too_many_arguments)] // each arg is load-bearing with distinct lifetime/ownership; an options struct would obscure call-site flow
fn walk_specifiers(
    source: &[u8],
    clause: Node<'_>,
    file_nid: &str,
    target_stem: &str,
    str_path: &str,
    relation: &str,
    context: &str,
    line: u32,
    edges: &mut Vec<Edge>,
) {
    let mut cur = clause.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let sub = cur.node();
        match sub.kind() {
            "named_imports" => {
                // `import { Foo, Bar } from '...'`
                let mut ncur = sub.walk();
                if ncur.goto_first_child() {
                    loop {
                        let spec = ncur.node();
                        if spec.kind() == "import_specifier"
                            && let Some(name_node) = spec.child_by_field_name("name")
                        {
                            let sym = read_text_owned(name_node, source);
                            edges.push(make_edge(
                                file_nid,
                                &make_id(&[target_stem, &sym]),
                                relation,
                                Some(context),
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
            "export_specifier" => {
                // `export { Foo } from '...'`. tree-sitter-typescript exposes
                // the identifier under field `name`, but older grammars
                // (and the JavaScript variant) may not — fall back to the
                // first `identifier` / `property_identifier` child.
                let mut name_node = sub.child_by_field_name("name");
                if name_node.is_none() {
                    let mut nc = sub.walk();
                    if nc.goto_first_child() {
                        loop {
                            let kind = nc.node().kind();
                            if matches!(kind, "identifier" | "property_identifier") {
                                name_node = Some(nc.node());
                                break;
                            }
                            if !nc.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                if let Some(name_node) = name_node {
                    let sym = read_text_owned(name_node, source);
                    edges.push(make_edge(
                        file_nid,
                        &make_id(&[target_stem, &sym]),
                        relation,
                        Some(context),
                        str_path,
                        line,
                    ));
                }
            }
            _ => {}
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
}

// ── Java ──────────────────────────────────────────────────────────────────────

/// Emit an `imports` edge for a Java `import_declaration` node.
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

/// Flatten a Java `scoped_identifier` chain into a dot-separated string (e.g. `"com.example.Foo"`).
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

/// Emit an `imports` edge for a C/C++ `#include` preprocessor node.
///
/// Resolves quoted includes to their canonical filesystem path when possible;
/// falls back to the bare filename stem for system and unresolvable headers.
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

/// Resolve a quoted C `#include` path relative to the including file, returning the canonical path if it exists.
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

/// Emit an `imports` edge for a C# `using_directive` node.
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

/// Emit an `imports` edge for a Kotlin `import_header` node.
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

/// Emit an `imports` edge for a Scala `import_declaration` node.
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

/// Emit an `imports` edge for a PHP `use_declaration` node.
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

/// Emit an `imports` edge for a Lua `require` call expression node.
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
    if let Some(raw_module) = find_require_module(&text) {
        let tgt_nid = resolve_lua_import_target(&raw_module, str_path);
        if !tgt_nid.is_empty() {
            edges.push(Edge {
                external: false,
                source: file_nid.to_string(),
                target: tgt_nid,
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

/// How many parent directories to probe when resolving a Lua `require()` to a
/// file on disk, so requires from nested files still find a package root above.
const LUA_MAX_PROBE_LEVELS: usize = 6;

/// Resolve a Lua `require()` module name to a node id (#1075).
///
/// Lua module names use dots as path separators: `require("pkg.b")` looks for
/// `pkg/b.lua` (or `pkg/b/init.lua`) relative to a package root. The importing
/// file's directory is probed and walked upward looking for a matching file on
/// disk; when found, the returned id matches the file node id `extract_generic`
/// assigns that file (`make_id1(path)`), so the edge lands on a real node. When
/// nothing matches, the full dotted module name's id is returned so cross-file
/// resolution can still complete via the symbol-resolution pass instead of the
/// edge being dropped entirely.
#[must_use]
fn resolve_lua_import_target(raw_module: &str, str_path: &str) -> String {
    if raw_module.is_empty() {
        return String::new();
    }
    let rel = raw_module.replace('.', "/");
    if let Some(start_dir) = std::path::Path::new(str_path).parent() {
        let mut probe = start_dir.to_path_buf();
        // Walk up a few levels so requires from nested files still resolve when
        // the package root is above the importing file.
        for _ in 0..LUA_MAX_PROBE_LEVELS {
            for suffix in [".lua", ".luau"] {
                let cand = probe.join(format!("{rel}{suffix}"));
                if cand.is_file() {
                    return make_id1(&cand.to_string_lossy());
                }
            }
            for suffix in [".lua", ".luau"] {
                let cand = probe.join(&rel).join(format!("init{suffix}"));
                if cand.is_file() {
                    return make_id1(&cand.to_string_lossy());
                }
            }
            match probe.parent() {
                Some(p) if p != probe => probe = p.to_path_buf(),
                _ => break,
            }
        }
    }
    make_id1(raw_module)
}

/// Extract the module string from a `require(...)` call in `text`, returning `None` if not found.
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

/// Emit an `imports` edge for a Swift `import_declaration` node.
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
