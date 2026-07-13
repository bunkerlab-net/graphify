//! PowerShell `.psd1` data-manifest extractor.

use super::read_text;
use crate::generic::walk::first_child_kind;
use crate::ids::make_id1;
use crate::types::{Edge, FileResult, Node};
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

/// `.psd1` manifest keys whose values are module names/paths treated as imports.
/// Mirrors `_PSD1_IMPORT_KEYS`.
static PSD1_IMPORT_KEYS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["RootModule", "NestedModules", "RequiredModules"]
        .into_iter()
        .collect()
});

/// Derive a bare module name from a raw string value: strip the path prefix and
/// extension (`MyModule.psm1` -> `MyModule`, `./sub/Util.psm1` -> `Util`).
/// Mirrors `_psd1_module_name`.
fn psd1_module_name(raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or("");
    let no_ext = basename.rsplit_once('.').map_or(basename, |(base, _)| base);
    no_ext.trim().to_string()
}

/// Recursively collect all `string_literal` text values (surrounding quotes
/// stripped) under `node`. Mirrors `_psd1_collect_string_literals`.
fn psd1_collect_string_literals(node: tree_sitter::Node<'_>, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "string_literal" {
        out.push(
            read_text(node, source)
                .trim_matches(['\'', '"'])
                .to_string(),
        );
        return;
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            psd1_collect_string_literals(cur.node(), source, out);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Like [`psd1_collect_string_literals`] but keeps each string's start byte so a
/// caller can distinguish strings nested inside a `hash_entry` from direct ones.
fn psd1_collect_string_nodes(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    out: &mut Vec<(usize, String)>,
) {
    if node.kind() == "string_literal" {
        out.push((
            node.start_byte(),
            read_text(node, source)
                .trim_matches(['\'', '"'])
                .to_string(),
        ));
        return;
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            psd1_collect_string_nodes(cur.node(), source, out);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// For `RequiredModules`: collect `ModuleName` values from hashtable specs and
/// record every string nested in a `hash_entry` (so the caller can treat only
/// the remaining direct strings as simple module names, and `ModuleVersion`
/// values never leak in). Mirrors the inner `find_modulename_entries`.
fn psd1_find_modulename_entries(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    module_names: &mut Vec<String>,
    inside_hash: &mut HashSet<usize>,
) {
    if node.kind() == "hash_entry" {
        let sub_key = first_child_kind(node, "key_expression");
        let sk_text = sub_key.map_or_else(String::new, |k| read_text(k, source).trim().to_string());
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "pipeline" {
                    let mut found = Vec::new();
                    psd1_collect_string_nodes(cur.node(), source, &mut found);
                    for (sb, s) in found {
                        inside_hash.insert(sb);
                        if sk_text == "ModuleName" {
                            module_names.push(s);
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return; // don't recurse further into this hash_entry
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            psd1_find_modulename_entries(cur.node(), source, module_names, inside_hash);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Push a `file -> module` `imports_from` edge for a raw `.psd1` module value.
fn add_psd1_import_edge(
    edges: &mut Vec<Edge>,
    file_nid: &str,
    str_path: &str,
    module_raw: &str,
    line: usize,
) {
    let name = psd1_module_name(module_raw);
    if name.is_empty() {
        return;
    }
    edges.push(Edge {
        external: false,
        source: file_nid.to_string(),
        target: make_id1(&name),
        relation: "imports_from".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: Some("import".to_string()),
        confidence_score: None,
        deferred: false,
        metadata: None,
    });
}

/// Walk a `.psd1` AST, emitting `imports_from` edges for `RootModule`,
/// `NestedModules`, and `RequiredModules` entries. Mirrors `walk_manifest`.
fn walk_psd1_manifest(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
) {
    if node.kind() != "hash_entry" {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                walk_psd1_manifest(cur.node(), source, file_nid, str_path, edges);
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        return;
    }
    let Some(key_node) = first_child_kind(node, "key_expression") else {
        return;
    };
    let key_text = read_text(key_node, source).trim().to_string();
    if !PSD1_IMPORT_KEYS.contains(key_text.as_str()) {
        return;
    }
    let line = node.start_position().row + 1;
    let Some(value_node) = first_child_kind(node, "pipeline") else {
        return;
    };
    match key_text.as_str() {
        "RootModule" | "NestedModules" => {
            let mut strings = Vec::new();
            psd1_collect_string_literals(value_node, source, &mut strings);
            for s in strings {
                add_psd1_import_edge(edges, file_nid, str_path, &s, line);
            }
        }
        "RequiredModules" => {
            // Two forms: plain 'Module' strings, and @{ ModuleName='Foo'; ... }
            // specs (follow only ModuleName; ModuleVersion etc. are excluded).
            let mut module_names = Vec::new();
            let mut inside_hash = HashSet::new();
            psd1_find_modulename_entries(value_node, source, &mut module_names, &mut inside_hash);
            let mut all_strings = Vec::new();
            psd1_collect_string_nodes(value_node, source, &mut all_strings);
            for (sb, s) in &all_strings {
                if !inside_hash.contains(sb) {
                    add_psd1_import_edge(edges, file_nid, str_path, s, line);
                }
            }
            for s in &module_names {
                add_psd1_import_edge(edges, file_nid, str_path, s, line);
            }
        }
        _ => {}
    }
}

/// Extract module dependency edges from a PowerShell `.psd1` manifest.
///
/// `.psd1` files are PowerShell data hashtables (syntactically valid PowerShell),
/// so tree-sitter parses them. Emits a file node plus `imports_from` edges for
/// every module named under `RootModule`, `NestedModules`, and `RequiredModules`
/// (both the plain-string and `@{ ModuleName=... }` forms). Mirrors
/// `extract_powershell_manifest`.
#[must_use]
pub fn extract_powershell_manifest(path: &Path) -> FileResult {
    let source = match std::fs::read(path) {
        Ok(s) => s,
        Err(e) => return FileResult::error(format!("powershell manifest read error: {e}")),
    };
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_powershell::LANGUAGE.into())
        .is_err()
    {
        return FileResult::error("tree_sitter_powershell language load failed");
    }
    let Some(tree) = parser.parse(&source, None) else {
        return FileResult::error("powershell manifest parse failed");
    };

    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    let nodes = vec![Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
        origin_file: None,
        node_type: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    walk_psd1_manifest(tree.root_node(), &source, &file_nid, &str_path, &mut edges);

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}
