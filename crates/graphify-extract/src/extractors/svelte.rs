//! Svelte and Astro extractors.
//!
//! Both files feed the full source to the JS/TS tree-sitter parser (which produces
//! an ERROR at the markup layer) and then rescue imports via regex over:
//! - `<script>` blocks for static imports
//! - `import('./X.svelte')` dynamic imports anywhere in the file
//! - Astro `---...---` frontmatter for static imports

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::generic::extract_generic;
use crate::ids::make_id1;
use crate::lang_configs;
use crate::tsconfig::{load_tsconfig_aliases, resolve_js_module_path};
use crate::types::{Edge, FileResult, Node};

// ── Regex patterns ────────────────────────────────────────────────────────────

#[allow(clippy::expect_used)] // literal patterns
static DYNAMIC_IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"import\(\s*['"]([^'"]+)['"]\s*\)"#).expect("static dynamic import regex")
});

#[allow(clippy::expect_used)]
static SCRIPT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<script\b[^>]*>([\s\S]*?)</script\s*>").expect("static script regex")
});

#[allow(clippy::expect_used)]
static STATIC_IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"import\s+(?:[^'"`;\n]+?\s+from\s+)?['"]([^'"]+)['"]"#)
        .expect("static import regex")
});

#[allow(clippy::expect_used)]
static FRONTMATTER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A\s*---\s*\r?\n([\s\S]*?)\r?\n---\s*(?:\r?\n|\z)")
        .expect("static frontmatter regex")
});

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Resolve a Svelte/Astro import specifier to a `(nid, stub_path)` pair.
///
/// Relative paths are joined to the importing file's directory and normalised; tsconfig path
/// aliases are expanded; bare module names are reduced to their last path segment. The returned
/// `stub_path` is used as the `source_file` for the import-target node.
fn resolve_import_id(raw: &str, path: &Path) -> (String, String) {
    if raw.starts_with('.') {
        let dir = path.parent().unwrap_or(path);
        let joined = dir.join(raw);
        // Normalize the path (collapse ../ etc)
        let normalised = joined
            .components()
            .fold(std::path::PathBuf::new(), |mut acc, c| {
                match c {
                    std::path::Component::ParentDir => {
                        acc.pop();
                    }
                    std::path::Component::CurDir => {}
                    other => acc.push(other),
                }
                acc
            });
        let resolved = resolve_js_module_path(&normalised);
        let stub = resolved.to_string_lossy().into_owned();
        (make_id1(&stub), stub)
    } else {
        // Check tsconfig aliases
        let aliases = load_tsconfig_aliases(path.parent().unwrap_or(path));
        let mut resolved_alias: Option<std::path::PathBuf> = None;
        for (alias_prefix, alias_base) in &aliases {
            if raw == alias_prefix || raw.starts_with(&format!("{alias_prefix}/")) {
                let rest = raw[alias_prefix.len()..].trim_start_matches('/');
                let joined = std::path::Path::new(alias_base).join(rest);
                let normalised =
                    joined
                        .components()
                        .fold(std::path::PathBuf::new(), |mut acc, c| {
                            match c {
                                std::path::Component::ParentDir => {
                                    acc.pop();
                                }
                                std::path::Component::CurDir => {}
                                other => acc.push(other),
                            }
                            acc
                        });
                resolved_alias = Some(resolve_js_module_path(&normalised));
                break;
            }
        }
        if let Some(alias_path) = resolved_alias {
            let stub = alias_path.to_string_lossy().into_owned();
            (make_id1(&stub), stub)
        } else {
            // External module: use last segment
            let module_name = raw.split('/').next_back().unwrap_or(raw);
            if module_name.is_empty() {
                (String::new(), raw.to_string())
            } else {
                (make_id1(module_name), raw.to_string())
            }
        }
    }
}

/// Fix up a static relative import: .js → .ts, .jsx → .tsx
fn fixup_static_relative(raw: &str, path: &Path) -> (String, String) {
    if raw.starts_with('.') {
        let dir = path.parent().unwrap_or(path);
        let joined = dir.join(raw);
        let normalised = joined
            .components()
            .fold(std::path::PathBuf::new(), |mut acc, c| {
                match c {
                    std::path::Component::ParentDir => {
                        acc.pop();
                    }
                    std::path::Component::CurDir => {}
                    other => acc.push(other),
                }
                acc
            });
        let resolved = if normalised.extension().is_some_and(|e| e == "js") {
            let ts = normalised.with_extension("ts");
            if ts.exists() { ts } else { normalised }
        } else if normalised.extension().is_some_and(|e| e == "jsx") {
            let tsx = normalised.with_extension("tsx");
            if tsx.exists() { tsx } else { normalised }
        } else {
            normalised
        };
        let stub = resolved.to_string_lossy().into_owned();
        (make_id1(&stub), stub)
    } else {
        let aliases = load_tsconfig_aliases(path.parent().unwrap_or(path));
        let mut resolved_alias: Option<std::path::PathBuf> = None;
        for (alias_prefix, alias_base) in &aliases {
            if raw == alias_prefix || raw.starts_with(&format!("{alias_prefix}/")) {
                let rest = raw[alias_prefix.len()..].trim_start_matches('/');
                let joined = std::path::Path::new(alias_base).join(rest);
                resolved_alias = Some(joined);
                break;
            }
        }
        if let Some(alias_path) = resolved_alias {
            // Route the aliased path through `resolve_js_module_path` so the
            // same `.js` → `.ts` / `.jsx` → `.tsx` fallback used for
            // relative imports applies to aliased ones too.
            let resolved = resolve_js_module_path(&alias_path);
            let stub = resolved.to_string_lossy().into_owned();
            (make_id1(&stub), stub)
        } else {
            let module_name = raw.split('/').next_back().unwrap_or(raw);
            if module_name.is_empty() {
                (String::new(), raw.to_string())
            } else {
                (make_id1(module_name), raw.to_string())
            }
        }
    }
}

/// Bundle of source-side identifiers shared by every Svelte import-edge insert.
struct SvelteImportEdge<'a> {
    file_node_id: &'a str,
    node_id: String,
    raw: &'a str,
    stub_source_file: String,
    relation: &'a str,
    str_path: &'a str,
}

/// Append an import edge and, if needed, an import-target stub node to the result.
///
/// Deduplicates by `existing_ids`. Creates a stub file node for the target when it is not
/// already present, allowing the graph to reference files not yet extracted.
fn add_import_edge(
    result: &mut FileResult,
    existing_ids: &mut HashSet<String>,
    args: SvelteImportEdge<'_>,
) {
    let SvelteImportEdge {
        file_node_id,
        node_id,
        raw,
        stub_source_file,
        relation,
        str_path,
    } = args;
    if node_id.is_empty() {
        return;
    }
    if existing_ids.contains(&node_id) {
        result.edges.push(Edge {
            source: file_node_id.to_string(),
            target: node_id,
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.to_string(),
            source_location: None,
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    } else {
        result.nodes.push(Node {
            id: node_id.clone(),
            label: raw.to_string(),
            file_type: "code".to_string(),
            source_file: stub_source_file,
            source_location: None,
            metadata: None,
        });
        result.edges.push(Edge {
            source: file_node_id.to_string(),
            target: node_id.clone(),
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.to_string(),
            source_location: None,
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
        existing_ids.insert(node_id);
    }
}

// ── extract_svelte ────────────────────────────────────────────────────────────

/// Extract imports from `.svelte` files: script-block via JS AST + template regex fallback.
#[must_use]
pub fn extract_svelte(path: &Path) -> FileResult {
    let mut result = extract_generic(path, &lang_configs::JAVASCRIPT);

    let Ok(src) = std::fs::read_to_string(path) else {
        return result;
    };

    let str_path = path.to_string_lossy().into_owned();
    let file_node_id = make_id1(&str_path);
    let mut existing_ids: HashSet<String> = result.nodes.iter().map(|n| n.id.clone()).collect();

    // Dynamic imports: import('./X.svelte')
    for cap in DYNAMIC_IMPORT_RE.captures_iter(&src) {
        let raw = cap.get(1).map_or("", |m| m.as_str());
        if raw.is_empty() {
            continue;
        }
        let (node_id, stub_source_file) = resolve_import_id(raw, path);
        add_import_edge(
            &mut result,
            &mut existing_ids,
            SvelteImportEdge {
                file_node_id: &file_node_id,
                node_id,
                raw,
                stub_source_file,
                relation: "dynamic_import",
                str_path: &str_path,
            },
        );
    }

    // Static imports inside <script> blocks
    for script_cap in SCRIPT_RE.captures_iter(&src) {
        let script_body = script_cap.get(1).map_or("", |m| m.as_str());
        for imp_cap in STATIC_IMPORT_RE.captures_iter(script_body) {
            let raw = imp_cap.get(1).map_or("", |m| m.as_str());
            if raw.is_empty() {
                continue;
            }
            let (node_id, stub_source_file) = fixup_static_relative(raw, path);
            add_import_edge(
                &mut result,
                &mut existing_ids,
                SvelteImportEdge {
                    file_node_id: &file_node_id,
                    node_id,
                    raw,
                    stub_source_file,
                    relation: "imports_from",
                    str_path: &str_path,
                },
            );
        }
    }

    result
}

// ── extract_astro ─────────────────────────────────────────────────────────────

/// Extract imports from `.astro` files: frontmatter (TS) + template regex fallback.
#[must_use]
pub fn extract_astro(path: &Path) -> FileResult {
    let mut result = extract_generic(path, &lang_configs::JAVASCRIPT);

    let Ok(src) = std::fs::read_to_string(path) else {
        return result;
    };

    let str_path = path.to_string_lossy().into_owned();
    let file_node_id = make_id1(&str_path);
    let mut existing_ids: HashSet<String> = result.nodes.iter().map(|n| n.id.clone()).collect();

    // Dynamic imports anywhere in the file
    for cap in DYNAMIC_IMPORT_RE.captures_iter(&src) {
        let raw = cap.get(1).map_or("", |m| m.as_str());
        if raw.is_empty() {
            continue;
        }
        let (node_id, stub_source_file) = resolve_import_id(raw, path);
        add_import_edge(
            &mut result,
            &mut existing_ids,
            SvelteImportEdge {
                file_node_id: &file_node_id,
                node_id,
                raw,
                stub_source_file,
                relation: "dynamic_import",
                str_path: &str_path,
            },
        );
    }

    // Static imports: frontmatter + <script> blocks
    let mut regions: Vec<&str> = Vec::new();
    if let Some(fm_cap) = FRONTMATTER_RE.captures(&src)
        && let Some(m) = fm_cap.get(1)
    {
        regions.push(m.as_str());
    }
    for script_cap in SCRIPT_RE.captures_iter(&src) {
        if let Some(m) = script_cap.get(1) {
            regions.push(m.as_str());
        }
    }

    for region in regions {
        for imp_cap in STATIC_IMPORT_RE.captures_iter(region) {
            let raw = imp_cap.get(1).map_or("", |m| m.as_str());
            if raw.is_empty() {
                continue;
            }
            let (node_id, stub_source_file) = fixup_static_relative(raw, path);
            add_import_edge(
                &mut result,
                &mut existing_ids,
                SvelteImportEdge {
                    file_node_id: &file_node_id,
                    node_id,
                    raw,
                    stub_source_file,
                    relation: "imports_from",
                    str_path: &str_path,
                },
            );
        }
    }

    result
}
