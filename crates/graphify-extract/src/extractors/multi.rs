//! Multi-file extraction orchestrator.
//!
//! Mirrors Python `extract()` from `extract.py`:
//! - Per-file dispatch via extension (or `.blade.php` suffix)
//! - Cache integration (graphify-cache)
//! - Parallel extraction via rayon for large batches
//! - Cross-file Python import resolution
//! - Cross-file Java import resolution
//! - Cross-file `raw_call` resolution
//! - ID relativisation (absolute → project-relative)
//! - `source_file` field relativisation

// Source file labels use lowercase extensions; case-insensitive comparison
// would misidentify e.g. ".PY" which does not exist in practice.
#![allow(clippy::case_sensitive_file_extension_comparisons)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::Value;

use crate::extractors::{
    extract_apex, extract_astro, extract_bash, extract_blade, extract_c, extract_cpp,
    extract_csharp, extract_csproj, extract_dart, extract_delphi_form, extract_dm, extract_dmf,
    extract_dmi, extract_dmm, extract_elixir, extract_fortran, extract_go, extract_groovy,
    extract_java, extract_js, extract_json, extract_julia, extract_kotlin, extract_lazarus_form,
    extract_lazarus_package, extract_lua, extract_markdown, extract_mcp_config, extract_objc,
    extract_package_manifest, extract_pascal, extract_php, extract_powershell,
    extract_powershell_manifest, extract_python, extract_razor, extract_ruby, extract_rust,
    extract_scala, extract_sln, extract_slnx, extract_sql, extract_svelte, extract_swift,
    extract_terraform, extract_verilog, extract_zig, is_mcp_config_path,
};
use crate::ids::make_id1;
use crate::import_handlers::make_edge;
use crate::types::{Edge, ExtractOutput, FileResult, Node, RawCall};

const PARALLEL_THRESHOLD: usize = 20;

// ── Dispatch table ────────────────────────────────────────────────────────────

type ExtractFn = fn(&Path) -> FileResult;

/// Return the per-language extractor function for a given file path, or `None` for unknown types.
///
/// Blade templates are identified by the `.blade.php` suffix before the extension is checked, so
/// that `foo.blade.php` routes to `extract_blade` rather than `extract_php`. All other languages
/// are dispatched solely on the file extension.
fn get_extractor(path: &Path) -> Option<ExtractFn> {
    // Blade templates: checked by suffix before extension
    let name = path.file_name().map_or("", |n| n.to_str().unwrap_or(""));
    if name.ends_with(".blade.php") {
        return Some(extract_blade);
    }
    // MCP config files (.mcp.json, claude_desktop_config.json, ...) are routed
    // by filename before generic .json dispatch so they get MCP-aware nodes
    // (servers, commands, packages, env vars) instead of opaque JSON keys.
    if is_mcp_config_path(path) {
        return Some(extract_mcp_config);
    }
    // Package manifests (apm.yml/pyproject.toml/go.mod/pom.xml) -> a canonical
    // package node + depends_on edges, by filename before generic suffix dispatch
    // (#1377). apm.yml would otherwise be a .yml document handled by the LLM.
    if graphify_detect::is_package_manifest_path(path) {
        return Some(extract_package_manifest);
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "py" => Some(extract_python),
        "js" | "jsx" | "mjs" | "ts" | "tsx" | "vue" => Some(extract_js),
        "go" => Some(extract_go),
        "rs" => Some(extract_rust),
        "java" => Some(extract_java),
        "groovy" | "gradle" => Some(extract_groovy),
        "c" | "h" => Some(extract_c),
        "cpp" | "cc" | "cxx" | "hpp" => Some(extract_cpp),
        "rb" => Some(extract_ruby),
        "cs" => Some(extract_csharp),
        "kt" | "kts" => Some(extract_kotlin),
        "scala" => Some(extract_scala),
        "php" => Some(extract_php),
        "swift" => Some(extract_swift),
        "lua" | "luau" | "toc" => Some(extract_lua),
        "zig" => Some(extract_zig),
        "ps1" | "psm1" => Some(extract_powershell),
        "psd1" => Some(extract_powershell_manifest),
        "ex" | "exs" => Some(extract_elixir),
        "m" | "mm" => Some(extract_objc),
        "jl" => Some(extract_julia),
        "f" | "F" | "f90" | "F90" | "f95" | "F95" | "f03" | "F03" | "f08" | "F08" => {
            Some(extract_fortran)
        }
        "svelte" => Some(extract_svelte),
        "astro" => Some(extract_astro),
        "dart" => Some(extract_dart),
        "v" | "sv" | "svh" => Some(extract_verilog),
        "sql" => Some(extract_sql),
        "md" | "mdx" | "qmd" => Some(extract_markdown),
        "pas" | "pp" | "dpr" | "dpk" | "lpr" | "inc" => Some(extract_pascal),
        "dfm" => Some(extract_delphi_form),
        "lfm" => Some(extract_lazarus_form),
        "lpk" => Some(extract_lazarus_package),
        "sh" | "bash" => Some(extract_bash),
        "json" => Some(extract_json),
        "dm" | "dme" => Some(extract_dm),
        "dmi" => Some(extract_dmi),
        "dmm" => Some(extract_dmm),
        "dmf" => Some(extract_dmf),
        "sln" => Some(extract_sln),
        "slnx" => Some(extract_slnx),
        "cls" | "trigger" => Some(extract_apex),
        "tf" | "tfvars" | "hcl" => Some(extract_terraform),
        "csproj" | "fsproj" | "vbproj" => Some(extract_csproj),
        "razor" | "cshtml" => Some(extract_razor),
        _ => None,
    }
}

// ── Cache helpers (thin wrappers around graphify-cache) ───────────────────────

/// Serialise a `FileResult` to a `serde_json::Value` suitable for caching.
///
/// Converts nodes, edges, and `raw_calls` to JSON arrays. Used as the write side of the
/// graphify-cache pair; see `value_to_file_result` for the read side.
fn file_result_to_value(result: &FileResult) -> Value {
    let nodes: Vec<Value> = result
        .nodes
        .iter()
        .map(|n| serde_json::to_value(n).unwrap_or(Value::Null))
        .collect();
    let edges: Vec<Value> = result
        .edges
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    let raw_calls: Vec<Value> = result
        .raw_calls
        .iter()
        .map(|rc| {
            serde_json::json!({
                "caller_nid": rc.caller_nid,
                "callee": rc.callee,
                "is_member_call": rc.is_member_call,
                "source_file": rc.source_file,
                "source_location": rc.source_location,
                "receiver": rc.receiver,
            })
        })
        .collect();
    serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "raw_calls": raw_calls,
    })
}

/// Deserialise a cached `serde_json::Value` back into a `FileResult`.
///
/// Missing or malformed sub-fields silently fall back to empty `Vec`s.
/// Counterpart to `file_result_to_value`.
fn value_to_file_result(v: &Value) -> FileResult {
    let nodes = v
        .get("nodes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|n| serde_json::from_value::<Node>(n.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let edges = v
        .get("edges")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|e| serde_json::from_value::<Edge>(e.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let raw_calls = v
        .get("raw_calls")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|rc| {
                    Some(RawCall {
                        caller_nid: rc.get("caller_nid")?.as_str()?.to_string(),
                        callee: rc.get("callee")?.as_str()?.to_string(),
                        is_member_call: rc
                            .get("is_member_call")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        source_file: rc
                            .get("source_file")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        source_location: rc
                            .get("source_location")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        // `receiver` (#1356) reads back as `None` when absent.
                        // Safe without a Swift cache bypass or schema-version
                        // check: the AST cache is namespaced by crate version
                        // (`cache/ast/v{version}/` via graphify-cache's
                        // EXTRACTOR_VERSION), so a pre-`receiver` entry sits
                        // under an older version dir `load_cached` never reads,
                        // invalidated by the version bump that shipped the field.
                        receiver: rc
                            .get("receiver")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    FileResult {
        nodes,
        edges,
        raw_calls,
        error: None,
    }
}

// ── Extract a single file (with cache) ───────────────────────────────────────

/// File suffixes whose per-file AST extraction is never cached: their cross-file
/// import resolution depends on sibling files that can appear or change between
/// runs, so a cached result would serve a stale (unresolved) import edge.
/// Mirrors Python `_JS_CACHE_BYPASS_SUFFIXES`.
const JS_CACHE_BYPASS_SUFFIXES: [&str; 7] = ["js", "jsx", "mjs", "ts", "tsx", "vue", "svelte"];

/// Extract a single file, returning a cached result when available.
///
/// Looks up the on-disk AST cache first; on a miss, dispatches to the language-specific
/// extractor and writes the result back to the cache. Files with no matching extractor
/// return an empty `FileResult` rather than an error.
fn extract_single_file(path: &Path, effective_root: &Path) -> FileResult {
    // JS/TS files bypass the AST cache so workspace/sibling import resolution is
    // recomputed each run (#9a7dbfb): a result cached while a sibling was absent
    // would otherwise pin a stale unresolved import edge.
    let bypass_cache = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| JS_CACHE_BYPASS_SUFFIXES.contains(&ext));

    if !bypass_cache && let Some(v) = graphify_cache::load_cached(path, effective_root, "ast") {
        return value_to_file_result(&v);
    }

    let Some(extractor) = get_extractor(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: None,
        };
    };

    let result = extractor(path);
    if !bypass_cache && result.error.is_none() {
        let v = file_result_to_value(&result);
        // best-effort save; ignore failures
        let _ = graphify_cache::save_cached(path, &v, effective_root, "ast");
    }
    result
}

// ── Cross-file Python import resolution helpers ───────────────────────────────

/// Recursively walk a Python AST collecting `from X import Y` statements.
///
/// On finding an `import_from_statement`, resolves the source module to a known stem via
/// `bare_to_qualified`, then emits `uses` edges from each local class to each imported symbol
/// that is present in `stem_to_entities`. Mirrors Python `_walk_imports` from `extract.py`.
/// Shared state threaded through every [`walk_imports`] recursion.
struct ImportWalkCtx<'a> {
    path: &'a Path,
    stem_to_entities: &'a HashMap<String, HashMap<String, String>>,
    bare_to_qualified: &'a HashMap<String, String>,
    local_classes: &'a [String],
    str_path: &'a str,
    new_edges: &'a mut Vec<Edge>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Python's import_from_statement variants
fn walk_imports(ctx: &mut ImportWalkCtx<'_>, node: tree_sitter::Node<'_>, source: &[u8]) {
    if node.kind() == "import_from_statement" {
        let mut target_fq: Option<String> = None;
        let mut past_import_kw = false;
        let mut imported_names: Vec<String> = Vec::new();
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "relative_import" {
                    let mut rc = child.walk();
                    if rc.goto_first_child() {
                        loop {
                            let sub = rc.node();
                            if sub.kind() == "dotted_name" {
                                let raw =
                                    std::str::from_utf8(&source[sub.start_byte()..sub.end_byte()])
                                        .unwrap_or("");
                                let bare = raw.split('.').next_back().unwrap_or("").to_string();
                                let candidate = ctx
                                    .path
                                    .parent()
                                    .unwrap_or(ctx.path)
                                    .join(format!("{bare}.py"));
                                target_fq = Some(crate::ids::file_stem(&candidate));
                                break;
                            }
                            if !rc.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                    break;
                }
                if child.kind() == "dotted_name" && target_fq.is_none() {
                    let raw = std::str::from_utf8(&source[child.start_byte()..child.end_byte()])
                        .unwrap_or("");
                    let bare = raw.split('.').next_back().unwrap_or("");
                    target_fq = ctx.bare_to_qualified.get(bare).cloned();
                }
                if child.kind() == "import" {
                    past_import_kw = true;
                } else if past_import_kw {
                    if child.kind() == "dotted_name" {
                        imported_names.push(
                            std::str::from_utf8(&source[child.start_byte()..child.end_byte()])
                                .unwrap_or("")
                                .to_string(),
                        );
                    } else if child.kind() == "aliased_import"
                        && let Some(name_node) = child.child_by_field_name("name")
                    {
                        imported_names.push(
                            std::str::from_utf8(
                                &source[name_node.start_byte()..name_node.end_byte()],
                            )
                            .unwrap_or("")
                            .to_string(),
                        );
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }

        let Some(fq) = target_fq else { return };
        let Some(entities) = ctx.stem_to_entities.get(&fq) else {
            return;
        };
        let line = node.start_position().row + 1;
        for name in &imported_names {
            if let Some(tgt_nid) = entities.get(name) {
                for src_class_nid in ctx.local_classes {
                    ctx.new_edges.push(Edge {
                        external: false,
                        source: src_class_nid.clone(),
                        target: tgt_nid.clone(),
                        relation: "uses".to_string(),
                        confidence: "INFERRED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 0.8,
                        context: None,
                        confidence_score: None,
                    });
                }
            }
        }
        return;
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_imports(ctx, cur.node(), source);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Recursively walk a Java AST collecting `import` declarations and resolving them to graph edges.
///
/// On finding an `import_declaration`, extracts the class name (or second-to-last component for
/// static method imports), looks it up in `name_to_ids`, and emits `imports` edges from the
/// current file node to any matching class nodes. Wildcard imports (`.*`) are silently skipped.
/// Mirrors Python `_walk_java` from `extract.py`.
fn walk_java(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    file_nid: &str,
    path: &Path,
    name_to_ids: &HashMap<String, Vec<String>>,
    new_edges: &mut Vec<Edge>,
    seen_pairs: &mut std::collections::HashSet<(String, String)>,
) {
    if node.kind() == "import_declaration" {
        let raw = std::str::from_utf8(&source[node.start_byte()..node.end_byte()])
            .unwrap_or("")
            .trim()
            .to_string();
        let body = raw
            .trim_start_matches("import")
            .trim()
            .trim_end_matches(';')
            .trim()
            .trim_start_matches("static ")
            .trim()
            .to_string();
        if body.ends_with(".*") {
            return;
        }
        let parts: Vec<&str> = body.split('.').collect();
        if parts.is_empty() {
            return;
        }
        let last = parts.last().copied().unwrap_or("");
        // If last part is lowercase, try second-to-last (method static import)
        let class_name = if last.chars().next().is_some_and(char::is_lowercase) && parts.len() >= 2
        {
            parts[parts.len() - 2]
        } else {
            last
        };
        let at_line = node.start_position().row + 1;
        for tgt_nid in name_to_ids.get(class_name).into_iter().flatten() {
            if tgt_nid == file_nid {
                continue;
            }
            let key = (file_nid.to_string(), tgt_nid.clone());
            if seen_pairs.insert(key) {
                new_edges.push(Edge {
                    external: false,
                    source: file_nid.to_string(),
                    target: tgt_nid.clone(),
                    relation: "imports".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: path.to_string_lossy().into_owned(),
                    source_location: Some(format!("L{at_line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: Some(1.0),
                });
            }
        }
        return;
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_java(
                cur.node(),
                source,
                file_nid,
                path,
                name_to_ids,
                new_edges,
                seen_pairs,
            );
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

// ── Cross-file Python import resolution ──────────────────────────────────────

/// Emit `uses` edges connecting Python classes to the symbols they import from other files.
///
/// Two-pass: first builds a map of (file-qualified-stem → label → nid) and
/// (bare stem → qualified stem); then re-parses each Python file to find
/// `from X import Y` statements and emit edges. Mirrors Python `_resolve_cross_file_imports`.
fn resolve_cross_file_python_imports(per_file: &[FileResult], paths: &[PathBuf]) -> Vec<Edge> {
    let mut probe = tree_sitter::Parser::new();
    if probe
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .is_err()
    {
        return vec![];
    }
    drop(probe);

    let (stem_to_entities, bare_to_qualified) = build_python_symbol_maps(per_file);
    let work: Vec<(&FileResult, &PathBuf)> = per_file.iter().zip(paths.iter()).collect();
    let init_parser = || -> tree_sitter::Parser {
        let mut p = tree_sitter::Parser::new();
        let _ = p.set_language(&tree_sitter_python::LANGUAGE.into());
        p
    };
    if work.len() >= PARALLEL_THRESHOLD {
        work.par_iter()
            .map_init(init_parser, |parser, (result, path)| {
                python_per_file_edges(result, path, parser, &stem_to_entities, &bare_to_qualified)
            })
            .reduce(Vec::new, |mut a, b| {
                a.extend(b);
                a
            })
    } else {
        let mut parser = init_parser();
        work.iter()
            .flat_map(|(result, path)| {
                python_per_file_edges(
                    result,
                    path,
                    &mut parser,
                    &stem_to_entities,
                    &bare_to_qualified,
                )
            })
            .collect()
    }
}

/// Pass 1: build `(stem → {label → nid})` + `(bare stem → qualified stem)` maps.
fn build_python_symbol_maps(
    per_file: &[FileResult],
) -> (
    HashMap<String, HashMap<String, String>>,
    HashMap<String, String>,
) {
    use crate::ids::file_stem;
    let mut stem_to_entities: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut bare_to_qualified: HashMap<String, String> = HashMap::new();
    for result in per_file {
        for node in &result.nodes {
            if node.source_file.is_empty() {
                continue;
            }
            let label = &node.label;
            if label.is_empty()
                || label.ends_with(')')
                || label.to_lowercase().ends_with(".py")
                || label.starts_with('_')
                || node.file_type == "rationale"
            {
                continue;
            }
            let src_path = PathBuf::from(&node.source_file);
            let fq_stem = file_stem(&src_path);
            stem_to_entities
                .entry(fq_stem.clone())
                .or_default()
                .insert(label.clone(), node.id.clone());
            let bare = src_path
                .file_stem()
                .map_or(String::new(), |s| s.to_string_lossy().into_owned());
            bare_to_qualified.entry(bare).or_insert(fq_stem);
        }
    }
    (stem_to_entities, bare_to_qualified)
}

/// Pass 2: per-file Python parse + import-edge emission.
fn python_per_file_edges(
    result: &FileResult,
    path: &Path,
    parser: &mut tree_sitter::Parser,
    stem_to_entities: &HashMap<String, HashMap<String, String>>,
    bare_to_qualified: &HashMap<String, String>,
) -> Vec<Edge> {
    use crate::ids::file_stem;
    let mut local_edges: Vec<Edge> = Vec::new();
    let str_path = path.to_string_lossy().into_owned();
    let this_stem = file_stem(path);
    let this_file_nid = make_id1(&str_path);
    let local_classes: Vec<String> = result
        .nodes
        .iter()
        .filter(|n| {
            n.source_file == str_path
                && !n.label.ends_with(')')
                && !n.label.to_lowercase().ends_with(".py")
                && n.id != this_file_nid
                && n.id != make_id1(&this_stem)
                && n.file_type != "rationale"
        })
        .map(|n| n.id.clone())
        .collect();
    if local_classes.is_empty() {
        return local_edges;
    }
    let Ok(source) = std::fs::read(path) else {
        return local_edges;
    };
    let Some(tree) = parser.parse(&source, None) else {
        return local_edges;
    };
    let mut import_ctx = ImportWalkCtx {
        path,
        stem_to_entities,
        bare_to_qualified,
        local_classes: &local_classes,
        str_path: &str_path,
        new_edges: &mut local_edges,
    };
    walk_imports(&mut import_ctx, tree.root_node(), &source);
    local_edges
}

// ── Cross-file Java import resolution ────────────────────────────────────────

/// Emit `imports` edges by resolving Java `import` statements across all extracted files.
///
/// Two-pass: first builds a map of (class-name → [nid]) from all capitalised node labels;
/// then re-parses each `.java` file to find `import_declaration` nodes and emit edges.
/// Mirrors Python `_resolve_cross_file_java_imports`.
#[allow(clippy::too_many_lines)]
fn resolve_cross_file_java_imports(per_file: &[FileResult], paths: &[PathBuf]) -> Vec<Edge> {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .is_err()
    {
        return vec![];
    }

    // Pass 1: class-name → [node_id]
    let mut name_to_ids: HashMap<String, Vec<String>> = HashMap::new();
    for result in per_file {
        for node in &result.nodes {
            let label = &node.label;
            if label.is_empty()
                || node.source_file.is_empty()
                || label.ends_with(')')
                || label.to_lowercase().ends_with(".java")
            {
                continue;
            }
            if !label
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() && c.is_uppercase())
            {
                continue;
            }
            name_to_ids
                .entry(label.clone())
                .or_default()
                .push(node.id.clone());
        }
    }

    // Pass 2: resolve imports — fan out across Rayon. Per-file work is
    // independent; we drop the seed parser and give each worker its own.
    // `seen_pairs` is partitioned per-file (each thread accumulates its
    // own pairs); the final dedupe runs sequentially after the parallel
    // reduce so edge ordering matches the sequential implementation
    // wherever it would have been preserved.
    drop(parser);

    let init_parser = || -> tree_sitter::Parser {
        let mut p = tree_sitter::Parser::new();
        let _ = p.set_language(&tree_sitter_java::LANGUAGE.into());
        p
    };

    let per_file_edges = |path: &PathBuf, parser: &mut tree_sitter::Parser| -> Vec<Edge> {
        let file_nid = make_id1(&path.to_string_lossy());
        let Ok(source) = std::fs::read(path) else {
            return Vec::new();
        };
        let Some(tree) = parser.parse(&source, None) else {
            return Vec::new();
        };
        let mut local_edges = Vec::new();
        let mut local_seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        walk_java(
            tree.root_node(),
            &source,
            &file_nid,
            path,
            &name_to_ids,
            &mut local_edges,
            &mut local_seen,
        );
        local_edges
    };

    let collected: Vec<Edge> = if paths.len() >= PARALLEL_THRESHOLD {
        paths
            .par_iter()
            .map_init(init_parser, |parser, path| per_file_edges(path, parser))
            .reduce(Vec::new, |mut a, b| {
                a.extend(b);
                a
            })
    } else {
        let mut parser = init_parser();
        paths
            .iter()
            .flat_map(|p| per_file_edges(p, &mut parser))
            .collect()
    };

    // Global dedupe: per-file `local_seen` only guards within a single
    // file, but the original sequential code shared `seen_pairs` across
    // every file. Recreate that property with a final pass over the
    // merged Vec to drop later duplicates.
    let mut new_edges: Vec<Edge> = Vec::with_capacity(collected.len());
    let mut seen_pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for e in collected {
        let key = (e.source.clone(), e.target.clone());
        if seen_pairs.insert(key) {
            new_edges.push(e);
        }
    }
    new_edges
}

/// Result of cross-file JS/TS default-import resolution (#6dc23db).
struct JsDefaultResolution {
    /// `imports` edges wiring an importer file node to the origin symbol of a
    /// default export, even when the local binding is renamed.
    edges: Vec<Edge>,
    /// `(caller_file_node_id, local_binding_lowercased) -> origin symbol node id`,
    /// so a call through a renamed default-import binding (`import mk from
    /// './foo'; mk()`) resolves to the origin during cross-file call resolution.
    aliases: HashMap<(String, String), String>,
}

/// The tree-sitter grammar for a JS/TS file, by extension (vue/others skipped).
fn js_grammar_for(path: &Path) -> Option<tree_sitter::Language> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Some("tsx") => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        Some("js" | "jsx" | "mjs" | "cjs") => Some(tree_sitter_javascript::LANGUAGE.into()),
        _ => None,
    }
}

/// UTF-8 slice of a node's source span (empty on invalid UTF-8).
fn js_node_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Local name of a default export, or `None` for an anonymous default.
///
/// Handles `export default class Foo {}` / `export default function foo() {}`
/// (name on the `declaration` field) and `export default Foo` (identifier on
/// the `value` field). Mirrors graphify-py `_js_default_export_name`.
fn js_default_export_name(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut c = node.walk();
    if !node.children(&mut c).any(|ch| ch.kind() == "default") {
        return None;
    }
    if let Some(decl) = node.child_by_field_name("declaration") {
        return decl
            .child_by_field_name("name")
            .map(|n| js_node_text(n, source).to_string());
    }
    let value = node.child_by_field_name("value")?;
    (value.kind() == "identifier").then(|| js_node_text(value, source).to_string())
}

/// Local binding of a default import — the `Foo` in `import Foo from './x'`
/// (also the leading binding of `import Foo, { Bar } from './x'`). Mirrors
/// graphify-py `_js_default_import_name`.
fn js_default_import_name(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut c = node.walk();
    let clause = node
        .children(&mut c)
        .find(|ch| ch.kind() == "import_clause")?;
    let mut cc = clause.walk();
    clause
        .children(&mut cc)
        .find(|sub| sub.kind() == "identifier")
        .map(|id| js_node_text(id, source).to_string())
}

/// The source-module string literal (`'./x'`) of an import/export statement.
fn js_import_source(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut c = node.walk();
    let s = node.children(&mut c).find(|ch| ch.kind() == "string")?;
    Some(
        js_node_text(s, source)
            .trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ' ')
            .to_string(),
    )
}

/// A default import occurrence: `(file index, local binding, source string, line)`.
type JsDefaultImport = (usize, String, String, u32);

/// Default-export names (by file index) and default imports gathered per file.
struct JsDefaultFacts {
    export_name: HashMap<usize, String>,
    imports: Vec<JsDefaultImport>,
}

/// Parse each JS/TS file once, collecting its default-export name (by file
/// index) and its default imports. Files without a JS/TS grammar or that fail to
/// read/parse are skipped.
fn collect_js_default_facts(paths: &[PathBuf]) -> JsDefaultFacts {
    let mut export_name: HashMap<usize, String> = HashMap::new();
    let mut imports: Vec<JsDefaultImport> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let Some(lang) = js_grammar_for(path) else {
            continue;
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang).is_err() {
            continue;
        }
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            match node.kind() {
                "export_statement" => {
                    if let Some(name) = js_default_export_name(node, &source) {
                        export_name.entry(i).or_insert(name);
                    }
                }
                "import_statement" => {
                    if let Some(local) = js_default_import_name(node, &source)
                        && let Some(src) = js_import_source(node, &source)
                    {
                        let line = u32::try_from(node.start_position().row)
                            .unwrap_or(0)
                            .saturating_add(1);
                        imports.push((i, local, src, line));
                    }
                }
                _ => {}
            }
            let mut c = node.walk();
            stack.extend(node.children(&mut c));
        }
    }
    JsDefaultFacts {
        export_name,
        imports,
    }
}

/// Resolve JS/TS default imports to the origin symbol of the matching default
/// export across files (#6dc23db).
///
/// graphify-py threads default imports/exports through its
/// `_collect_js_symbol_resolution_facts` pass; the Rust port resolves JS imports
/// per-file, so this adds the cross-file default case as a focused resolver
/// parallel to [`resolve_cross_file_python_imports`] /
/// [`resolve_cross_file_java_imports`]. Runs after id remapping so it works in
/// the final node-id space. `all_nodes` is the post-remap node set.
fn resolve_js_default_imports(
    all_nodes: &[Node],
    paths: &[PathBuf],
    root: &Path,
) -> JsDefaultResolution {
    use crate::ids::file_node_id;

    let file_nid_of = |path: &Path| -> String {
        let rel = relativise_under_root(path, root).unwrap_or_else(|| path.to_path_buf());
        file_node_id(&rel)
    };

    // (file_node_id, normalised label) -> node id, so a default-export name
    // resolves to the concrete symbol node in that file. The label is normalised
    // the same way the call resolver normalises call labels (strip a trailing
    // `()` and a leading `.`) so a function export (`makeFoo`, stored as the node
    // label `makeFoo()`) still matches the bare export name.
    let mut by_file_label: HashMap<(String, String), String> = HashMap::new();
    for n in all_nodes {
        if n.source_file.is_empty() || n.label.is_empty() {
            continue;
        }
        let sf = PathBuf::from(&n.source_file);
        let file_nid = if sf.is_absolute() {
            file_nid_of(&sf)
        } else {
            file_node_id(&sf)
        };
        let label = n.label.trim_end_matches("()").trim_start_matches('.');
        if label.is_empty() {
            continue;
        }
        by_file_label
            .entry((file_nid, label.to_string()))
            .or_insert_with(|| n.id.clone());
    }

    // Per file: default-export name + default imports.
    let JsDefaultFacts {
        export_name,
        imports,
    } = collect_js_default_facts(paths);

    // Match each canonicalised path to its index, so a resolved import target
    // maps back to the file whose default export we recorded.
    let mut idx_by_path: HashMap<PathBuf, usize> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        idx_by_path.entry(p.clone()).or_insert(i);
        if let Ok(c) = p.canonicalize() {
            idx_by_path.entry(c).or_insert(i);
        }
    }

    let mut edges = Vec::new();
    let mut aliases = HashMap::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (imp_idx, local, raw, line) in imports {
        let importer = &paths[imp_idx];
        let str_path = importer.to_string_lossy();
        let (_, resolved) = crate::generic::resolve_js_import_target(&raw, &str_path);
        let Some(resolved) = resolved else { continue };
        let tgt_idx = idx_by_path
            .get(&resolved)
            .or_else(|| {
                resolved
                    .canonicalize()
                    .ok()
                    .and_then(|c| idx_by_path.get(&c))
            })
            .copied();
        let Some(tgt_idx) = tgt_idx else { continue };
        let Some(name) = export_name.get(&tgt_idx) else {
            continue;
        };
        let tgt_file_nid = file_nid_of(&paths[tgt_idx]);
        let Some(origin) = by_file_label.get(&(tgt_file_nid, name.clone())) else {
            continue;
        };
        let importer_nid = file_nid_of(importer);
        if seen.insert((importer_nid.clone(), origin.clone())) {
            edges.push(make_edge(
                &importer_nid,
                origin,
                "imports",
                Some("import"),
                &str_path,
                line,
            ));
        }
        aliases.insert((importer_nid, local.to_lowercase()), origin.clone());
    }

    JsDefaultResolution { edges, aliases }
}

/// Per-file JS/TS export/import specifier facts used to resolve barrel
/// re-export chains to their origin symbols (#barrel-resolution). Collected by
/// [`collect_js_reexport_facts`].
#[derive(Default)]
struct JsReexportFile {
    /// `export { S as P } from './x'` → `(public, source_raw, source_name)`.
    reexports: Vec<(String, String, String)>,
    /// `export * from './x'` → `source_raw`.
    star_sources: Vec<String>,
    /// `export { L as P }` (no `from`) → `(public, local)`.
    local_reexports: Vec<(String, String)>,
    /// `export const X = …` → `X` (the public exported binding name).
    exported_const_names: Vec<String>,
    /// `import { I as L } from './x'` → `local → (source_raw, imported)`.
    named_imports: HashMap<String, (String, String)>,
    /// `const B = A` / `export const B = A` (bare-identifier RHS) → `alias → target`.
    local_aliases: HashMap<String, String>,
    /// Named imports as consumer facts: `(local_binding, source_raw, imported, line)`.
    consumer_imports: Vec<(String, String, String, u32)>,
}

/// Extract `(name, alias)` from an `import_specifier` / `export_specifier`.
fn js_spec_name_alias(
    spec: tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<(String, Option<String>)> {
    let name = spec.child_by_field_name("name").or_else(|| {
        let mut c = spec.walk();
        spec.children(&mut c)
            .find(|n| matches!(n.kind(), "identifier" | "property_identifier"))
    })?;
    let alias = spec
        .child_by_field_name("alias")
        .map(|a| js_node_text(a, source).to_string());
    Some((js_node_text(name, source).to_string(), alias))
}

/// Record `const B = A` bare-identifier aliases from a `lexical_declaration`.
fn collect_js_lexical_aliases(node: tree_sitter::Node<'_>, source: &[u8], f: &mut JsReexportFile) {
    let mut cur = node.walk();
    for d in node.children(&mut cur) {
        if d.kind() == "variable_declarator"
            && let Some(name) = d.child_by_field_name("name")
            && let Some(value) = d.child_by_field_name("value")
            && value.kind() == "identifier"
        {
            f.local_aliases.insert(
                js_node_text(name, source).to_string(),
                js_node_text(value, source).to_string(),
            );
        }
    }
}

/// Record named imports (`import { I as L } from './x'`) from an `import_statement`.
fn collect_js_import_stmt(node: tree_sitter::Node<'_>, source: &[u8], f: &mut JsReexportFile) {
    let Some(src) = js_import_source(node, source) else {
        return;
    };
    let line = u32::try_from(node.start_position().row)
        .unwrap_or(0)
        .saturating_add(1);
    let mut cur = node.walk();
    for child in node.children(&mut cur) {
        if child.kind() != "import_clause" {
            continue;
        }
        let mut cc = child.walk();
        for sub in child.children(&mut cc) {
            if sub.kind() != "named_imports" {
                continue;
            }
            let mut nc = sub.walk();
            for spec in sub.children(&mut nc) {
                if spec.kind() == "import_specifier"
                    && let Some((name, alias)) = js_spec_name_alias(spec, source)
                {
                    let local = alias.unwrap_or_else(|| name.clone());
                    f.named_imports
                        .insert(local.clone(), (src.clone(), name.clone()));
                    f.consumer_imports.push((local, src.clone(), name, line));
                }
            }
        }
    }
}

/// Record re-exports / star re-exports / local re-exports / exported consts
/// from an `export_statement`.
fn collect_js_export_stmt(node: tree_sitter::Node<'_>, source: &[u8], f: &mut JsReexportFile) {
    let src = js_import_source(node, source);
    let mut cur = node.walk();
    let children: Vec<tree_sitter::Node<'_>> = node.children(&mut cur).collect();
    let export_clause = children
        .iter()
        .find(|c| c.kind() == "export_clause")
        .copied();
    let has_namespace = children.iter().any(|c| c.kind() == "namespace_export");
    let lexical = children
        .iter()
        .find(|c| c.kind() == "lexical_declaration")
        .copied();

    if let Some(clause) = export_clause {
        let mut cc = clause.walk();
        for spec in clause.children(&mut cc) {
            if spec.kind() == "export_specifier"
                && let Some((name, alias)) = js_spec_name_alias(spec, source)
            {
                let public = alias.unwrap_or_else(|| name.clone());
                match &src {
                    Some(s) => f.reexports.push((public, s.clone(), name)),
                    None => f.local_reexports.push((public, name)),
                }
            }
        }
    } else if let Some(s) = &src {
        if !has_namespace {
            f.star_sources.push(s.clone());
        }
    } else if let Some(lex) = lexical {
        collect_js_lexical_aliases(lex, source, f);
        let mut lc = lex.walk();
        for d in lex.children(&mut lc) {
            if d.kind() == "variable_declarator"
                && let Some(nn) = d.child_by_field_name("name")
            {
                f.exported_const_names
                    .push(js_node_text(nn, source).to_string());
            }
        }
    }
}

/// Parse each JS/TS file once, collecting its barrel re-export facts (indexed by
/// `paths` position). Files without a JS/TS grammar are recorded as empty.
fn collect_js_reexport_facts(paths: &[PathBuf]) -> Vec<JsReexportFile> {
    let mut out: Vec<JsReexportFile> = Vec::with_capacity(paths.len());
    for path in paths {
        let mut f = JsReexportFile::default();
        if let Some(lang) = js_grammar_for(path)
            && let Ok(source) = std::fs::read(path)
        {
            let mut parser = tree_sitter::Parser::new();
            if parser.set_language(&lang).is_ok()
                && let Some(tree) = parser.parse(&source, None)
            {
                let root = tree.root_node();
                let mut cur = root.walk();
                for stmt in root.children(&mut cur) {
                    match stmt.kind() {
                        "export_statement" => collect_js_export_stmt(stmt, &source, &mut f),
                        "import_statement" => collect_js_import_stmt(stmt, &source, &mut f),
                        "lexical_declaration" => collect_js_lexical_aliases(stmt, &source, &mut f),
                        _ => {}
                    }
                }
            }
        }
        out.push(f);
    }
    out
}

/// Re-export chain resolver over the collected [`JsReexportFile`] facts.
struct ReexportResolver<'a> {
    facts: &'a [JsReexportFile],
    idx_by_path: &'a HashMap<PathBuf, usize>,
    paths: &'a [PathBuf],
    file_nids: &'a [String],
    by_file_label: &'a HashMap<(String, String), String>,
}

impl ReexportResolver<'_> {
    /// `true` when `name` is declared as a real symbol node in file `idx`.
    fn is_declared(&self, idx: usize, name: &str) -> bool {
        self.by_file_label
            .contains_key(&(self.file_nids[idx].clone(), name.to_string()))
    }

    /// Resolve an import-source string (`'./x'`) to the `paths` index it targets.
    fn resolve_src(&self, file_idx: usize, src_raw: &str) -> Option<usize> {
        let str_path = self.paths[file_idx].to_string_lossy();
        let (_, resolved) = crate::generic::resolve_js_import_target(src_raw, &str_path);
        let resolved = resolved?;
        self.idx_by_path
            .get(&resolved)
            .or_else(|| {
                resolved
                    .canonicalize()
                    .ok()
                    .and_then(|c| self.idx_by_path.get(&c))
            })
            .copied()
    }

    /// Resolve `name` exported from file `file_idx` to its origin
    /// `(file_idx, declared_name)`, following named/aliased/star re-exports,
    /// local aliases, and named imports. `visited` guards against cycles.
    fn resolve(
        &self,
        file_idx: usize,
        name: &str,
        visited: &mut HashSet<(usize, String)>,
    ) -> Option<(usize, String)> {
        if !visited.insert((file_idx, name.to_string())) {
            return None;
        }
        let f = &self.facts[file_idx];
        for (public, src_raw, src_name) in &f.reexports {
            if public == name
                && let Some(tgt) = self.resolve_src(file_idx, src_raw)
                && let Some(r) = self.resolve(tgt, src_name, visited)
            {
                return Some(r);
            }
        }
        for (public, local) in &f.local_reexports {
            if public == name
                && local != name
                && let Some(r) = self.resolve(file_idx, local, visited)
            {
                return Some(r);
            }
        }
        if let Some(target) = f.local_aliases.get(name)
            && let Some(r) = self.resolve(file_idx, target, visited)
        {
            return Some(r);
        }
        if let Some((src_raw, imported)) = f.named_imports.get(name)
            && let Some(tgt) = self.resolve_src(file_idx, src_raw)
            && let Some(r) = self.resolve(tgt, imported, visited)
        {
            return Some(r);
        }
        for src_raw in &f.star_sources {
            if let Some(tgt) = self.resolve_src(file_idx, src_raw)
                && let Some(r) = self.resolve(tgt, name, visited)
            {
                return Some(r);
            }
        }
        if self.is_declared(file_idx, name) {
            return Some((file_idx, name.to_string()));
        }
        None
    }

    /// File→file `re_exports` edges for every barrel export that resolves to an
    /// origin file other than the barrel itself.
    fn reexport_edges(&self) -> Vec<Edge> {
        let mut edges = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for (idx, f) in self.facts.iter().enumerate() {
            let barrel_nid = &self.file_nids[idx];
            let str_path = self.paths[idx].to_string_lossy();
            let publics = f
                .reexports
                .iter()
                .map(|(p, _, _)| p)
                .chain(f.local_reexports.iter().map(|(p, _)| p))
                .chain(f.exported_const_names.iter());
            for public in publics {
                let mut visited = HashSet::new();
                if let Some((origin_idx, _)) = self.resolve(idx, public, &mut visited)
                    && origin_idx != idx
                    && seen.insert((barrel_nid.clone(), self.file_nids[origin_idx].clone()))
                {
                    edges.push(make_edge(
                        barrel_nid,
                        &self.file_nids[origin_idx],
                        "re_exports",
                        Some("re-export"),
                        &str_path,
                        1,
                    ));
                }
            }
            for src_raw in &f.star_sources {
                if let Some(tgt) = self.resolve_src(idx, src_raw)
                    && tgt != idx
                    && seen.insert((barrel_nid.clone(), self.file_nids[tgt].clone()))
                {
                    edges.push(make_edge(
                        barrel_nid,
                        &self.file_nids[tgt],
                        "re_exports",
                        Some("re-export"),
                        &str_path,
                        1,
                    ));
                }
            }
        }
        edges
    }

    /// Consumer `imports` edges + call aliases for named imports that travel
    /// through a barrel to an origin symbol in a different file.
    fn consumer_import_edges(&self) -> (Vec<Edge>, HashMap<(String, String), String>) {
        let mut edges = Vec::new();
        let mut aliases: HashMap<(String, String), String> = HashMap::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for (idx, f) in self.facts.iter().enumerate() {
            let consumer_nid = &self.file_nids[idx];
            let str_path = self.paths[idx].to_string_lossy();
            for (local, src_raw, imported, line) in &f.consumer_imports {
                let Some(barrel_idx) = self.resolve_src(idx, src_raw) else {
                    continue;
                };
                let mut visited = HashSet::new();
                let Some((origin_idx, origin_name)) =
                    self.resolve(barrel_idx, imported, &mut visited)
                else {
                    continue;
                };
                // origin == directly-imported file ⇒ plain import handled per-file.
                if origin_idx == barrel_idx {
                    continue;
                }
                let Some(origin_sym) = self
                    .by_file_label
                    .get(&(self.file_nids[origin_idx].clone(), origin_name.clone()))
                else {
                    continue;
                };
                if seen.insert((consumer_nid.clone(), origin_sym.clone())) {
                    edges.push(make_edge(
                        consumer_nid,
                        origin_sym,
                        "imports",
                        Some("import"),
                        &str_path,
                        *line,
                    ));
                }
                aliases.insert(
                    (consumer_nid.clone(), local.to_lowercase()),
                    origin_sym.clone(),
                );
            }
        }
        (edges, aliases)
    }
}

/// Resolve JS/TS named/aliased/star barrel re-export chains to their origin
/// symbols, emitting file→file `re_exports` edges, consumer→origin `imports`
/// edges, and call aliases (so a call through a barrel-imported binding targets
/// the origin symbol). Mirrors the observable output of graphify-py's
/// `_collect_js_symbol_resolution_facts` / `_apply_symbol_resolution_facts`
/// barrel handling, integrated with the existing per-file resolution.
fn resolve_js_reexport_imports(
    all_nodes: &[Node],
    paths: &[PathBuf],
    root: &Path,
) -> JsDefaultResolution {
    use crate::ids::file_node_id;

    let file_nid_of = |path: &Path| -> String {
        let rel = relativise_under_root(path, root).unwrap_or_else(|| path.to_path_buf());
        file_node_id(&rel)
    };
    let mut by_file_label: HashMap<(String, String), String> = HashMap::new();
    for n in all_nodes {
        if n.source_file.is_empty() || n.label.is_empty() {
            continue;
        }
        let sf = PathBuf::from(&n.source_file);
        let file_nid = if sf.is_absolute() {
            file_nid_of(&sf)
        } else {
            file_node_id(&sf)
        };
        let label = n.label.trim_end_matches("()").trim_start_matches('.');
        if label.is_empty() {
            continue;
        }
        by_file_label
            .entry((file_nid, label.to_string()))
            .or_insert_with(|| n.id.clone());
    }

    let facts = collect_js_reexport_facts(paths);
    let mut idx_by_path: HashMap<PathBuf, usize> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        idx_by_path.entry(p.clone()).or_insert(i);
        if let Ok(c) = p.canonicalize() {
            idx_by_path.entry(c).or_insert(i);
        }
    }
    let file_nids: Vec<String> = paths.iter().map(|p| file_nid_of(p)).collect();
    let resolver = ReexportResolver {
        facts: &facts,
        idx_by_path: &idx_by_path,
        paths,
        file_nids: &file_nids,
        by_file_label: &by_file_label,
    };

    let mut edges = resolver.reexport_edges();
    let (import_edges, aliases) = resolver.consumer_import_edges();
    edges.extend(import_edges);

    JsDefaultResolution { edges, aliases }
}

/// `(module_raw, [(imported_name, local_or_public_name)])` from a Python
/// `import_from_statement` (alias-aware, unlike on-disk-only `python_imported_names`).
fn python_import_from_specs(
    source: &[u8],
    node: tree_sitter::Node<'_>,
) -> Option<(String, Vec<(String, String)>)> {
    let module = node.child_by_field_name("module_name")?;
    let module_raw = js_node_text(module, source).to_string();
    let mut specs = Vec::new();
    let mut past_import = false;
    let mut cur = node.walk();
    for child in node.children(&mut cur) {
        match child.kind() {
            "import" => past_import = true,
            "dotted_name" if past_import => {
                let n = js_node_text(child, source).to_string();
                specs.push((n.clone(), n));
            }
            "aliased_import" if past_import => {
                if let Some(nn) = child.child_by_field_name("name") {
                    let imported = js_node_text(nn, source).to_string();
                    let local = child
                        .child_by_field_name("alias")
                        .map_or_else(|| imported.clone(), |a| js_node_text(a, source).to_string());
                    specs.push((imported, local));
                }
            }
            _ => {}
        }
    }
    Some((module_raw, specs))
}

/// Candidate file paths a relative Python module reference can resolve to,
/// against `from_path`. A `.foo` reference can name either a module file
/// (`foo.py`) or a package (`foo/__init__.py`); `from . import x` names the
/// current package's `__init__.py`. Returns an empty list for a non-relative
/// module. The caller picks the first candidate present in the scan set.
fn python_relative_module_candidates(from_path: &Path, module_raw: &str) -> Vec<PathBuf> {
    if !module_raw.starts_with('.') {
        return Vec::new();
    }
    let dots = module_raw.len() - module_raw.trim_start_matches('.').len();
    let module_name = module_raw.trim_start_matches('.');
    let Some(mut base) = from_path.parent().map(Path::to_path_buf) else {
        return Vec::new();
    };
    for _ in 0..dots.saturating_sub(1) {
        let Some(parent) = base.parent() else {
            return Vec::new();
        };
        base = parent.to_path_buf();
    }
    if module_name.is_empty() {
        return vec![base.join("__init__.py")];
    }
    let rel = module_name.replace('.', "/");
    vec![
        base.join(format!("{rel}.py")),
        base.join(&rel).join("__init__.py"),
    ]
}

/// Look up a path's `paths` index, falling back to its canonicalised form.
fn py_idx_of(idx_by_path: &HashMap<PathBuf, usize>, p: &Path) -> Option<usize> {
    idx_by_path
        .get(p)
        .or_else(|| p.canonicalize().ok().and_then(|c| idx_by_path.get(&c)))
        .copied()
}

/// Parse a Python file, returning its source bytes + tree.
fn parse_python_file(path: &Path) -> Option<(Vec<u8>, tree_sitter::Tree)> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    let source = std::fs::read(path).ok()?;
    let tree = parser.parse(&source, None)?;
    Some((source, tree))
}

/// `(init_idx, public_name) → (origin_idx, origin_name)` package re-export map.
type PyPkgReexports = HashMap<(usize, String), (usize, String)>;

/// Shared maps for Python package re-export resolution.
struct PyReexportResolver<'a> {
    paths: &'a [PathBuf],
    idx_by_path: &'a HashMap<PathBuf, usize>,
    file_nids: &'a [String],
    by_file_label: &'a HashMap<(String, String), String>,
}

impl PyReexportResolver<'_> {
    /// Scan every `__init__.py` for `from .sub import N as A`, building a
    /// `(init_idx, public) → (origin_idx, origin_name)` map and emitting
    /// file→file `re_exports` edges.
    fn pkg_reexports(&self) -> (PyPkgReexports, Vec<Edge>) {
        let mut map: PyPkgReexports = HashMap::new();
        let mut edges = Vec::new();
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        for (idx, path) in self.paths.iter().enumerate() {
            if path.file_name().and_then(|n| n.to_str()) != Some("__init__.py") {
                continue;
            }
            let Some((source, tree)) = parse_python_file(path) else {
                continue;
            };
            let mut cur = tree.root_node().walk();
            for stmt in tree.root_node().children(&mut cur) {
                if stmt.kind() != "import_from_statement" {
                    continue;
                }
                let Some((module_raw, specs)) = python_import_from_specs(&source, stmt) else {
                    continue;
                };
                let Some(sub_idx) = python_relative_module_candidates(path, &module_raw)
                    .iter()
                    .find_map(|cand| py_idx_of(self.idx_by_path, cand))
                else {
                    continue;
                };
                for (imported, public) in specs {
                    map.insert((idx, public), (sub_idx, imported));
                }
                if seen.insert((idx, sub_idx)) {
                    edges.push(make_edge(
                        &self.file_nids[idx],
                        &self.file_nids[sub_idx],
                        "re_exports",
                        Some("re-export"),
                        &path.to_string_lossy(),
                        1,
                    ));
                }
            }
        }
        (map, edges)
    }

    /// Resolve each `from pkg import N` against the package re-export map,
    /// emitting consumer→origin `imports` edges and call aliases.
    fn consumer_edges(
        &self,
        pkg_reexports: &PyPkgReexports,
    ) -> (Vec<Edge>, HashMap<(String, String), String>) {
        let mut edges = Vec::new();
        let mut aliases: HashMap<(String, String), String> = HashMap::new();
        let mut seen: HashSet<(usize, String)> = HashSet::new();
        for (idx, path) in self.paths.iter().enumerate() {
            let str_path = path.to_string_lossy();
            let Some((source, tree)) = parse_python_file(path) else {
                continue;
            };
            let mut cur = tree.root_node().walk();
            for stmt in tree.root_node().children(&mut cur) {
                if stmt.kind() != "import_from_statement" {
                    continue;
                }
                let Some((module_raw, specs)) = python_import_from_specs(&source, stmt) else {
                    continue;
                };
                if module_raw.starts_with('.') {
                    continue;
                }
                let Some(pkg_dir) =
                    crate::import_handlers::resolve_python_package_dir(&module_raw, &str_path)
                else {
                    continue;
                };
                let Some(init_idx) = py_idx_of(self.idx_by_path, &pkg_dir.join("__init__.py"))
                else {
                    continue;
                };
                for (imported, local) in specs {
                    let Some((origin_idx, origin_name)) = pkg_reexports.get(&(init_idx, imported))
                    else {
                        continue;
                    };
                    let label = origin_name.trim_end_matches("()").trim_start_matches('.');
                    let Some(origin_sym) = self
                        .by_file_label
                        .get(&(self.file_nids[*origin_idx].clone(), label.to_string()))
                    else {
                        continue;
                    };
                    if seen.insert((idx, origin_sym.clone())) {
                        edges.push(make_edge(
                            &self.file_nids[idx],
                            origin_sym,
                            "imports",
                            Some("import"),
                            &str_path,
                            1,
                        ));
                    }
                    aliases.insert(
                        (self.file_nids[idx].clone(), local.to_lowercase()),
                        origin_sym.clone(),
                    );
                }
            }
        }
        (edges, aliases)
    }
}

/// Resolve Python package re-exports (`pkg/__init__.py` doing
/// `from .sub import Name as Alias`) so a consumer's `from pkg import Alias`
/// (and calls through it) target the origin symbol. Mirrors the observable
/// output of graphify-py's `_collect_python_symbol_resolution_facts`.
fn resolve_python_reexport_imports(
    all_nodes: &[Node],
    paths: &[PathBuf],
    root: &Path,
) -> JsDefaultResolution {
    use crate::ids::file_node_id;

    let file_nid_of = |path: &Path| -> String {
        let rel = relativise_under_root(path, root).unwrap_or_else(|| path.to_path_buf());
        file_node_id(&rel)
    };
    let mut by_file_label: HashMap<(String, String), String> = HashMap::new();
    for n in all_nodes {
        if n.source_file.is_empty() || n.label.is_empty() {
            continue;
        }
        let sf = PathBuf::from(&n.source_file);
        let file_nid = if sf.is_absolute() {
            file_nid_of(&sf)
        } else {
            file_node_id(&sf)
        };
        let label = n.label.trim_end_matches("()").trim_start_matches('.');
        if !label.is_empty() {
            by_file_label
                .entry((file_nid, label.to_string()))
                .or_insert_with(|| n.id.clone());
        }
    }
    let mut idx_by_path: HashMap<PathBuf, usize> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        idx_by_path.entry(p.clone()).or_insert(i);
        if let Ok(c) = p.canonicalize() {
            idx_by_path.entry(c).or_insert(i);
        }
    }
    let file_nids: Vec<String> = paths.iter().map(|p| file_nid_of(p)).collect();
    let resolver = PyReexportResolver {
        paths,
        idx_by_path: &idx_by_path,
        file_nids: &file_nids,
        by_file_label: &by_file_label,
    };
    let (pkg_reexports, mut edges) = resolver.pkg_reexports();
    let (import_edges, aliases) = resolver.consumer_edges(&pkg_reexports);
    edges.extend(import_edges);
    JsDefaultResolution { edges, aliases }
}

/// Relativise `path` against `root`, falling back to canonicalising the path
/// when a lexical strip fails (e.g. the path is relative, or differs from
/// `root` only by a symlink such as macOS's `/var` → `/private/var`).
///
/// Mirrors Python's `path.relative_to(root)` with its
/// `path.resolve().relative_to(root)` fallback. Returns `None` only when the
/// path is genuinely outside `root`.
#[must_use]
fn relativise_under_root(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(rel.to_path_buf());
    }
    path.canonicalize()
        .ok()
        .and_then(|c| c.strip_prefix(root).map(Path::to_path_buf).ok())
}

/// Recursively collect the `package` declaration and `import`s (simple name ->
/// FQN, capitalised type imports only) from a parsed Java file. Mirrors the
/// inner `walk` in Python `_resolve_java_type_references`.
fn collect_java_pkg_imports(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    pkg: &mut String,
    imps: &mut HashMap<String, String>,
) {
    match node.kind() {
        "package_declaration" => {
            let txt = node.utf8_text(source).unwrap_or("");
            *pkg = txt
                .trim()
                .strip_prefix("package")
                .unwrap_or(txt)
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
        }
        "import_declaration" => {
            let txt = node.utf8_text(source).unwrap_or("");
            let stripped = txt
                .trim()
                .strip_prefix("import")
                .unwrap_or(txt)
                .trim()
                .trim_end_matches(';')
                .trim();
            let body = stripped.strip_prefix("static ").map_or(stripped, str::trim);
            if !body.ends_with(".*")
                && body.contains('.')
                && let Some(simple) = body.rsplit('.').next()
                && !simple.is_empty()
                && simple.chars().next().is_some_and(char::is_uppercase)
            {
                imps.insert(simple.to_string(), body.to_string());
            }
        }
        _ => {}
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            collect_java_pkg_imports(cur.node(), source, pkg, imps);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

// Java edge relations re-pointed from shadow stubs to real defs by
// `resolve_java_type_references`. `imports` is included so a file-level import
// edge that also landed on the shadow stub gets re-pointed too, leaving the stub
// unreferenced (and dropped). External/stdlib imports never resolve, so their
// edges correctly stay on their stub.
const JAVA_REPOINT_RELATIONS: &[&str] = &["implements", "inherits", "extends", "imports"];

/// Re-point dangling Java `implements`/`inherits`/`extends`/`imports` edges that
/// bare-name resolution left on sourceless shadow stubs, using each referencing
/// file's `import` statements (then its package) to disambiguate same-named types
/// across packages (#1318). Drops shadow stubs no edge references anymore.
///
/// Mirrors Python `_resolve_java_type_references`. Runs after id-disambiguation
/// and `rewire_unique_stub_nodes` (so it only handles the ambiguous remainder),
/// in the final node-id space; keyed by the absolute `source_file` strings the
/// nodes/edges still carry before the closing relativisation pass.
fn resolve_java_type_references(
    java_paths: &[PathBuf],
    all_nodes: &mut Vec<Node>,
    all_edges: &mut [Edge],
) {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .is_err()
    {
        return;
    }
    let mut pkg_by_file: HashMap<String, String> = HashMap::new();
    let mut imports_by_file: HashMap<String, HashMap<String, String>> = HashMap::new();
    for path in java_paths {
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut pkg = String::new();
        let mut imps: HashMap<String, String> = HashMap::new();
        collect_java_pkg_imports(tree.root_node(), &source, &mut pkg, &mut imps);
        let src = path.to_string_lossy().into_owned();
        pkg_by_file.insert(src.clone(), pkg);
        imports_by_file.insert(src, imps);
    }

    // FQN (`package.Class`) -> definition node id, for source-backed type-like defs.
    let mut fqn_to_id: HashMap<String, String> = HashMap::new();
    for n in all_nodes.iter() {
        if n.label.is_empty() || n.source_file.is_empty() || n.id.is_empty() {
            continue;
        }
        let Some(pkg) = pkg_by_file.get(&n.source_file) else {
            continue;
        };
        let first_upper = n.label.chars().next().is_some_and(char::is_uppercase);
        if !first_upper || n.label.ends_with(')') || n.label.ends_with(".java") {
            continue;
        }
        let fqn = if pkg.is_empty() {
            n.label.clone()
        } else {
            format!("{pkg}.{}", n.label)
        };
        fqn_to_id.entry(fqn).or_insert_with(|| n.id.clone());
    }

    // Bare shadow stubs: no source_file, capitalised (type-like) label.
    let stub_label: HashMap<String, String> = all_nodes
        .iter()
        .filter(|n| {
            !n.id.is_empty()
                && n.source_file.is_empty()
                && n.label.chars().next().is_some_and(char::is_uppercase)
        })
        .map(|n| (n.id.clone(), n.label.clone()))
        .collect();
    if stub_label.is_empty() {
        return;
    }

    let mut repointed_from: std::collections::HashSet<String> = std::collections::HashSet::new();
    for edge in all_edges.iter_mut() {
        if !JAVA_REPOINT_RELATIONS.contains(&edge.relation.as_str()) {
            continue;
        }
        let Some(label) = stub_label.get(&edge.target) else {
            continue;
        };
        let resolved: Option<String> = {
            let ref_file = edge.source_file.as_str();
            imports_by_file
                .get(ref_file)
                .and_then(|imps| imps.get(label))
                .and_then(|fqn| fqn_to_id.get(fqn))
                .or_else(|| {
                    // Same-package reference (no explicit import).
                    let pkg = pkg_by_file.get(ref_file).map_or("", String::as_str);
                    let fqn = if pkg.is_empty() {
                        label.clone()
                    } else {
                        format!("{pkg}.{label}")
                    };
                    fqn_to_id.get(&fqn)
                })
                .cloned()
        };
        if let Some(r) = resolved
            && r != edge.target
        {
            repointed_from.insert(std::mem::replace(&mut edge.target, r));
        }
    }
    if repointed_from.is_empty() {
        return;
    }

    // Drop shadow stubs that no edge references anymore.
    let still_referenced: std::collections::HashSet<&str> = all_edges
        .iter()
        .flat_map(|e| [e.source.as_str(), e.target.as_str()])
        .collect();
    all_nodes
        .retain(|n| !repointed_from.contains(&n.id) || still_referenced.contains(n.id.as_str()));
}

/// `_is_type_like_definition`: a real type def (not a method, not a qualified or
/// decorated reference). Mirrors the Python predicate.
fn is_type_like_definition(node: &Node) -> bool {
    let label = node.label.trim();
    !label.is_empty()
        && !label.ends_with(')')
        && !label.starts_with('.')
        && !label.contains('.')
        && node.file_type == "code"
}

/// Re-parse a Swift file's AST into a `local name -> type name` table, from
/// property declarations (type annotation, else constructor inference) and
/// function parameters. Feeds [`resolve_swift_member_calls`]. Rebuilt by
/// re-parsing (like the Java type-reference pass) rather than threaded through a
/// `FileResult` sidecar.
fn collect_swift_type_table(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    table: &mut HashMap<String, String>,
) {
    use crate::generic::references::{
        RefRole, swift_collect_type_refs, swift_constructor_type, swift_property_name,
        swift_property_type_node,
    };
    match node.kind() {
        "property_declaration" => {
            let mut prop_type: Option<String> = None;
            if let Some(anno) = swift_property_type_node(node) {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                swift_collect_type_refs(anno, source, false, &mut refs);
                prop_type = refs
                    .into_iter()
                    .find(|(_, r)| *r == RefRole::Direct)
                    .map(|(n, _)| n);
            }
            if prop_type.is_none() {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "call_expression"
                            && let Some(ctor) = swift_constructor_type(cur.node(), source)
                        {
                            prop_type = Some(ctor);
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if let (Some(name), Some(ty)) = (swift_property_name(node, source), prop_type) {
                table.insert(name, ty);
            }
        }
        "parameter" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let mut refs: Vec<(String, RefRole)> = Vec::new();
                swift_collect_type_refs(type_node, source, false, &mut refs);
                if let Some((ty, _)) = refs.into_iter().find(|(_, r)| *r == RefRole::Direct)
                    && let Some(name_node) = node.child_by_field_name("name")
                {
                    let pname = name_node.utf8_text(source).unwrap_or("");
                    if !pname.is_empty() {
                        table.insert(pname.to_string(), ty);
                    }
                }
            }
        }
        _ => {}
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            collect_swift_type_table(cur.node(), source, table);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Resolve cross-file Swift member calls (`recv.method()`) to the receiver's
/// real type definition (#1356). The shared call pass drops every
/// `is_member_call` (a bare method name collides across the corpus); this pass
/// types the receiver via the file's local type table (or treats an upper-cased
/// receiver as a type itself), then emits an edge ONLY when the type name
/// resolves to exactly one definition (god-node guard). Everything it adds is
/// INFERRED (type inference, not an explicit import).
#[allow(clippy::too_many_lines)] // linear: re-parse type tables, build indexes, resolve each member call
fn resolve_swift_member_calls(
    swift_paths: &[PathBuf],
    all_nodes: &[Node],
    all_edges: &mut Vec<Edge>,
    all_raw_calls: &[RawCall],
) {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .is_err()
    {
        return;
    }
    let mut type_table_by_file: HashMap<String, HashMap<String, String>> = HashMap::new();
    for path in swift_paths {
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut table: HashMap<String, String> = HashMap::new();
        collect_swift_type_table(tree.root_node(), &source, &mut table);
        type_table_by_file.insert(path.to_string_lossy().into_owned(), table);
    }
    if type_table_by_file.is_empty() {
        return;
    }

    let key = |s: &str| -> String {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase()
    };

    // A genuine type is the target of a `contains` edge from its file; bare type
    // references create same-label shadow nodes that are NOT contained, so this
    // keeps a shadow from making a real type name look ambiguous.
    let contained: std::collections::HashSet<&str> = all_edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| e.target.as_str())
        .collect();
    let mut type_def_nids: HashMap<String, Vec<String>> = HashMap::new();
    let mut node_by_id: HashMap<&str, &Node> = HashMap::new();
    for n in all_nodes {
        node_by_id.insert(n.id.as_str(), n);
        if !n.source_file.is_empty()
            && contained.contains(n.id.as_str())
            && is_type_like_definition(n)
        {
            type_def_nids
                .entry(key(n.label.as_str()))
                .or_default()
                .push(n.id.clone());
        }
    }

    // (type_node_id, method_key) -> method_node_id, from `method` edges.
    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    for e in all_edges.iter() {
        if e.relation == "method"
            && let Some(tnode) = node_by_id.get(e.target.as_str())
        {
            method_index.insert(
                (e.source.clone(), key(tnode.label.as_str())),
                e.target.clone(),
            );
        }
    }

    let mut existing_pairs: std::collections::HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if !rc.is_member_call || rc.callee.is_empty() || rc.caller_nid.is_empty() {
            continue;
        }
        let Some(receiver) = rc.receiver.as_deref() else {
            continue;
        };
        // An upper-cased receiver is itself a type (`Type.staticMethod()`,
        // `Singleton.shared.x()`); otherwise look it up in the declaring file's
        // local type table.
        let type_name = if receiver.chars().next().is_some_and(char::is_uppercase) {
            receiver.to_string()
        } else if let Some(t) = type_table_by_file
            .get(&rc.source_file)
            .and_then(|tbl| tbl.get(receiver))
        {
            t.clone()
        } else {
            continue;
        };
        let type_nid = match type_def_nids.get(&key(type_name.as_str())) {
            Some(defs) if defs.len() == 1 => &defs[0],
            _ => continue, // ambiguous or absent -> god-node guard
        };
        let (target, relation) =
            match method_index.get(&(type_nid.clone(), key(rc.callee.as_str()))) {
                Some(method) => (method.clone(), "calls"),
                None => (type_nid.clone(), "references"),
            };
        if target == rc.caller_nid
            || existing_pairs.contains(&(rc.caller_nid.clone(), target.clone()))
        {
            continue;
        }
        existing_pairs.insert((rc.caller_nid.clone(), target.clone()));
        new_edges.push(Edge {
            external: false,
            source: rc.caller_nid.clone(),
            target,
            relation: relation.to_string(),
            confidence: "INFERRED".to_string(),
            source_file: rc.source_file.clone(),
            source_location: Some(rc.source_location.clone()),
            weight: 1.0,
            context: Some("call".to_string()),
            confidence_score: Some(0.8),
        });
    }
    all_edges.extend(new_edges);
}

// ── Main extract() ────────────────────────────────────────────────────────────

/// Extract AST nodes and edges from a list of code files.
///
/// Two-pass process:
/// 1. Per-file structural extraction (classes, functions, imports) — parallel if ≥ 20 uncached
/// 2. Cross-file import + call resolution
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn extract(paths: &[PathBuf], cache_root: Option<&Path>) -> ExtractOutput {
    if paths.is_empty() {
        return ExtractOutput {
            nodes: vec![],
            edges: vec![],
            input_tokens: 0,
            output_tokens: 0,
        };
    }

    // Workspace package manifests/globs can change between repeated extractions
    // (e.g. a new package added) or during `watch`; clear the cache so each run
    // re-scans. Mirrors Python `extract()`'s `_WORKSPACE_PACKAGE_CACHE.clear()`.
    crate::workspace::clear_workspace_cache();

    // Infer common root for ID relativisation
    let root: PathBuf = {
        let inferred = if paths.len() == 1 {
            paths[0]
                .parent()
                .map_or_else(|| PathBuf::from("."), PathBuf::from)
        } else {
            let min_parts = paths
                .iter()
                .map(|p| p.components().count())
                .min()
                .unwrap_or(0);
            let mut common_len = 0usize;
            'outer: for i in 0..min_parts {
                let first = paths[0].components().nth(i);
                for p in paths.iter().skip(1) {
                    if p.components().nth(i) != first {
                        break 'outer;
                    }
                }
                common_len = i + 1;
            }
            if common_len == 0 {
                PathBuf::from(".")
            } else {
                paths[0].components().take(common_len).collect()
            }
        };
        // An explicit `cache_root` overrides the inferred prefix, matching
        // Python's `if cache_root is not None: root = cache_root`. The root
        // drives both cache keys and the #1033 file-node-id relativisation, so
        // a divergence here splits AST/semantic file nodes apart.
        let base = cache_root.map_or(inferred, Path::to_path_buf);
        base.canonicalize().unwrap_or(base)
    };

    let effective_root: &Path = cache_root.unwrap_or(&root);

    // Phase 1: extract per file (cached or fresh)
    let uncached_work: Vec<(usize, &PathBuf)> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| get_extractor(p).is_some())
        .collect();

    let mut per_file: Vec<FileResult> = paths.iter().map(|_| FileResult::default()).collect();

    if uncached_work.len() >= PARALLEL_THRESHOLD {
        // Parallel via rayon
        let results: Vec<(usize, FileResult)> = uncached_work
            .par_iter()
            .map(|(idx, path)| (*idx, extract_single_file(path, effective_root)))
            .collect();
        for (idx, result) in results {
            per_file[idx] = result;
        }
    } else {
        // Sequential
        for (idx, path) in &uncached_work {
            per_file[*idx] = extract_single_file(path, effective_root);
        }
    }

    // Cross-file Python import resolution — must run BEFORE per_file is
    // drained into `all_*`, otherwise `resolve_cross_file_*` sees empty
    // FileResults and emits no cross-module edges.
    let mut cross_edges: Vec<Edge> = Vec::new();
    let py_indices: Vec<usize> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| p.extension().is_some_and(|e| e == "py"))
        .map(|(i, _)| i)
        .collect();
    if !py_indices.is_empty() {
        let py_results: Vec<FileResult> = py_indices.iter().map(|&i| per_file[i].clone()).collect();
        let py_paths: Vec<PathBuf> = py_indices.iter().map(|&i| paths[i].clone()).collect();
        cross_edges.extend(resolve_cross_file_python_imports(&py_results, &py_paths));
    }

    // Cross-file Java import resolution
    let java_indices: Vec<usize> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| p.extension().is_some_and(|e| e == "java"))
        .map(|(i, _)| i)
        .collect();
    if !java_indices.is_empty() {
        let java_results: Vec<FileResult> =
            java_indices.iter().map(|&i| per_file[i].clone()).collect();
        let java_paths: Vec<PathBuf> = java_indices.iter().map(|&i| paths[i].clone()).collect();
        cross_edges.extend(resolve_cross_file_java_imports(&java_results, &java_paths));
    }

    let mut all_nodes: Vec<Node> = Vec::new();
    let mut all_edges: Vec<Edge> = Vec::new();
    let mut all_raw_calls: Vec<RawCall> = Vec::new();

    for result in &mut per_file {
        all_nodes.append(&mut result.nodes);
        all_edges.append(&mut result.edges);
        all_raw_calls.append(&mut result.raw_calls);
    }
    all_edges.extend(cross_edges);

    // Remap absolute file-node IDs to the canonical `{parent_dir}_{stem}` spec
    // form so (a) edge endpoints are stable across machines (#502) and (b) AST
    // file nodes match the IDs semantic subagents generate (#1033).
    let mut id_remap: HashMap<String, String> = HashMap::new();
    // Symbol node IDs embed the file stem the extractor saw as a prefix. For a
    // root-level file that stem picks up the absolute parent directory name, so
    // a symbol becomes `<rootdir>_main_run` while the file node correctly
    // relativises to `main` and the spec wants `main_run` — splitting the symbol
    // into AST/semantic ghosts (#1096). Relativise the symbol prefix the same
    // way, gated by `source_file` so two files sharing a prefix can't
    // cross-contaminate. Keyed by the path string the extractor recorded in
    // `source_file` → (old_prefix, new_prefix).
    let mut prefix_remap: HashMap<String, (String, String)> = HashMap::new();
    for path in paths {
        let old_id = make_id1(&path.to_string_lossy());
        // Resolve relative-to-root; a lexical strip can fail (path is relative, or
        // differs from `root` only by a symlink), so fall back to canonicalising —
        // mirrors Python's `resolve().relative_to(root)` fallback.
        let Some(rel) = relativise_under_root(path, &root) else {
            continue;
        };
        let new_id = crate::ids::file_node_id(&rel);
        if old_id != new_id {
            id_remap.insert(old_id, new_id.clone());
        }
        // Import resolution (e.g. the pnpm `.`-package entry, #1083) canonicalises
        // the resolved path, which on macOS rewrites `/tmp` → `/private/tmp`. That
        // id differs from the input-path id keyed above, so an edge targeting the
        // canonical spelling would dangle off the relativised file node. Map the
        // canonical spelling to the same node so the resolved edge connects.
        if let Ok(canon) = path.canonicalize() {
            let canon_id = make_id1(&canon.to_string_lossy());
            if canon_id != new_id {
                id_remap.entry(canon_id).or_insert_with(|| new_id.clone());
            }
        }
        let old_pref = crate::ids::file_node_id(path);
        if old_pref != new_id {
            prefix_remap.insert(path.to_string_lossy().into_owned(), (old_pref, new_id));
        }
    }
    if !id_remap.is_empty() {
        for n in &mut all_nodes {
            if let Some(new_id) = id_remap.get(&n.id) {
                n.id = new_id.clone();
            }
        }
        for e in &mut all_edges {
            if let Some(new_id) = id_remap.get(&e.source) {
                e.source = new_id.clone();
            }
            if let Some(new_id) = id_remap.get(&e.target) {
                e.target = new_id.clone();
            }
        }
    }
    if !prefix_remap.is_empty() {
        let mut sym_remap: HashMap<String, String> = HashMap::new();
        for n in &all_nodes {
            if n.source_file.is_empty() {
                continue;
            }
            // Package (#1377) and Swift module (#1327) anchor nodes carry a
            // canonical name-keyed id (`pkg_<name>` / the shared module id) that
            // must stay identical across every manifest/file that references them,
            // so they are exempt from the file-stem prefix remap.
            if n.metadata
                .as_ref()
                .and_then(|m| m.get("type"))
                .and_then(Value::as_str)
                .is_some_and(|t| t == "package" || t == "module")
            {
                continue;
            }
            let Some((old_pref, new_pref)) = prefix_remap.get(&n.source_file) else {
                continue;
            };
            // IDs are make_id output (lowercase word chars + `_`), so slicing at
            // a byte offset is always on a char boundary.
            if n.id.len() > old_pref.len()
                && n.id.starts_with(old_pref.as_str())
                && n.id.as_bytes()[old_pref.len()] == b'_'
            {
                let new_nid = format!("{new_pref}{}", &n.id[old_pref.len()..]);
                if new_nid != n.id {
                    sym_remap.insert(n.id.clone(), new_nid);
                }
            }
        }
        if !sym_remap.is_empty() {
            for n in &mut all_nodes {
                if let Some(new_id) = sym_remap.get(&n.id) {
                    n.id = new_id.clone();
                }
            }
            for e in &mut all_edges {
                if let Some(new_id) = sym_remap.get(&e.source) {
                    e.source = new_id.clone();
                }
                if let Some(new_id) = sym_remap.get(&e.target) {
                    e.target = new_id.clone();
                }
            }
            // raw_calls carry caller_nid (a symbol id) consumed by the cross-file
            // call pass below — rewrite it too or those edges dangle on a stale
            // source (#1096).
            for rc in &mut all_raw_calls {
                if let Some(new_id) = sym_remap.get(&rc.caller_nid) {
                    rc.caller_nid = new_id.clone();
                }
            }
        }
    }

    // Disambiguate node IDs that collide across two or more distinct
    // source files (e.g. two `Program.cs` files in different directories).
    // Runs before cross-file call resolution so the call resolver sees
    // already-qualified IDs.
    crate::postprocess::disambiguate_colliding_node_ids(
        &mut all_nodes,
        &mut all_edges,
        &mut all_raw_calls,
        &root,
    );

    // Rewire cross-language inheritance stub nodes (no `source_file`) onto
    // a unique real definition with the same label. Drops the stub when
    // the rewire succeeds.
    crate::postprocess::rewire_unique_stub_nodes(&mut all_nodes, &mut all_edges);

    // Re-point dangling Java implements/inherits edges left on shadow stubs by
    // bare-name resolution, using imports for exact-package disambiguation
    // (#1318). After rewire_unique_stub_nodes so it only handles the ambiguous
    // remainder; before the closing source_file relativisation so node/edge
    // source_files still match the parsed Java file paths.
    let java_type_paths: Vec<PathBuf> = paths
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "java"))
        .cloned()
        .collect();
    if !java_type_paths.is_empty() {
        resolve_java_type_references(&java_type_paths, &mut all_nodes, &mut all_edges);
    }

    // Collapse Swift `extension Foo` nodes onto the canonical `class Foo`
    // declaration. Mirrors `_merge_swift_extensions` in graphify-py.
    crate::postprocess::merge_swift_extensions(paths, &mut all_nodes, &mut all_edges);

    // Cross-file JS/TS default-import resolution (#6dc23db). Runs in the final
    // node-id space (after remap/disambiguation); the `imports` edges feed the
    // import-evidence index below and the aliases let calls through a renamed
    // default binding resolve to the origin symbol.
    let js_default = resolve_js_default_imports(&all_nodes, paths, &root);
    all_edges.extend(js_default.edges);
    let mut js_default_aliases = js_default.aliases;
    // Cross-file JS/TS barrel re-export resolution: chain named/aliased/star
    // re-exports (and local-alias re-exports) to the origin symbol so consumer
    // imports + calls through a barrel target the real declaration.
    let js_reexport = resolve_js_reexport_imports(&all_nodes, paths, &root);
    all_edges.extend(js_reexport.edges);
    js_default_aliases.extend(js_reexport.aliases);
    // Cross-file Python package re-export resolution: `pkg/__init__.py` doing
    // `from .sub import N as A` lets `from pkg import A` (and calls through it)
    // target the origin symbol in `sub`.
    let py_reexport = resolve_python_reexport_imports(&all_nodes, paths, &root);
    all_edges.extend(py_reexport.edges);
    js_default_aliases.extend(py_reexport.aliases);

    // Cross-file call resolution via raw_calls
    // Build label → [nid] (skip rationale)
    let mut global_label_to_nids: HashMap<String, Vec<String>> = HashMap::new();
    for n in &all_nodes {
        if n.file_type == "rationale" {
            continue;
        }
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        if !normalised.is_empty() {
            global_label_to_nids
                .entry(normalised.to_lowercase())
                .or_default()
                .push(n.id.clone());
        }
    }

    // Import evidence indexes
    let mut file_to_symbol_imports: HashMap<String, std::collections::HashSet<String>> =
        HashMap::new();
    let mut file_to_module_imports: HashMap<String, std::collections::HashSet<String>> =
        HashMap::new();
    for e in &all_edges {
        if e.relation == "imports" {
            file_to_symbol_imports
                .entry(e.source.clone())
                .or_default()
                .insert(e.target.clone());
        } else if e.relation == "imports_from" {
            file_to_module_imports
                .entry(e.source.clone())
                .or_default()
                .insert(e.target.clone());
        }
    }

    // Map node → file_nid
    let mut nid_to_file_nid: HashMap<String, String> = HashMap::new();
    for n in &all_nodes {
        if n.source_file.is_empty() {
            continue;
        }
        let sf_path = PathBuf::from(&n.source_file);
        // Relativise the same way `id_remap` does so a symbol's file-nid matches
        // its (relativised) file node id — including the canonicalise fallback
        // for absolute paths that differ from `root` only by a symlink. Relative
        // source paths are used verbatim (mirrors Python).
        let sf_rel = if sf_path.is_absolute() {
            relativise_under_root(&sf_path, &root).unwrap_or(sf_path)
        } else {
            sf_path
        };
        nid_to_file_nid.insert(n.id.clone(), crate::ids::file_node_id(&sf_rel));
    }

    let mut existing_pairs: std::collections::HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    for rc in &all_raw_calls {
        // No built-in pre-filter here: the per-language extractors already drop
        // *unresolved* built-in calls at the source, so any raw_call that reaches
        // this cross-file pass is a genuine unresolved symbol. Filtering on the
        // name alone would wrongly suppress a project symbol that happens to
        // share a built-in name and resolves uniquely below.
        if rc.is_member_call {
            continue;
        }
        let callee_key = rc.callee.to_lowercase();
        let caller = &rc.caller_nid;
        let caller_file_nid = nid_to_file_nid.get(caller);
        // A renamed default-import binding (`import mk from './foo'; mk()`) aliases
        // the local name to the origin symbol; prefer that over global label
        // matching, since the local name has no node of its own (#6dc23db).
        let alias_tgt =
            caller_file_nid.and_then(|f| js_default_aliases.get(&(f.clone(), callee_key.clone())));
        let candidates: Vec<&String> = match alias_tgt {
            Some(t) => vec![t],
            None => global_label_to_nids
                .get(&callee_key)
                .map_or_else(Vec::new, |v| v.iter().collect()),
        };
        // Only resolve unambiguous matches
        if candidates.len() != 1 {
            continue;
        }
        let tgt = candidates[0];
        if tgt == caller {
            continue;
        }
        let pair = (caller.clone(), tgt.clone());
        if existing_pairs.contains(&pair) {
            continue;
        }

        let tgt_file_nid = nid_to_file_nid.get(tgt);
        let imported_symbols = caller_file_nid
            .and_then(|f| file_to_symbol_imports.get(f))
            .is_some_and(|s| s.contains(tgt));
        let imported_module = caller_file_nid
            .and_then(|f| file_to_module_imports.get(f))
            .zip(tgt_file_nid)
            .is_some_and(|(m, cfn)| m.contains(cfn));
        let has_import_evidence = imported_symbols || imported_module;

        let (confidence, confidence_score) = if has_import_evidence {
            ("EXTRACTED".to_string(), 1.0f64)
        } else {
            ("INFERRED".to_string(), 0.8f64)
        };

        existing_pairs.insert(pair);
        all_edges.push(Edge {
            external: false,
            source: caller.clone(),
            target: tgt.clone(),
            relation: "calls".to_string(),
            confidence,
            source_file: rc.source_file.clone(),
            source_location: Some(rc.source_location.clone()),
            weight: 1.0,
            context: Some("call".to_string()),
            confidence_score: Some(confidence_score),
        });
    }

    // Cross-file Swift member-call resolution (#1356): after the shared call pass
    // (node ids and caller_nids final) and before source_file relativisation (the
    // type-table re-parse keys on the absolute paths nodes/raw_calls still carry).
    let swift_paths: Vec<PathBuf> = paths
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "swift"))
        .cloned()
        .collect();
    if !swift_paths.is_empty() {
        resolve_swift_member_calls(&swift_paths, &all_nodes, &mut all_edges, &all_raw_calls);
    }

    // Relativise source_file fields
    for n in &mut all_nodes {
        let sf_path = PathBuf::from(&n.source_file);
        if sf_path.is_absolute()
            && let Ok(rel) = sf_path.strip_prefix(&root)
        {
            n.source_file = rel.to_string_lossy().into_owned();
        }
    }
    for e in &mut all_edges {
        let sf_path = PathBuf::from(&e.source_file);
        if sf_path.is_absolute()
            && let Ok(rel) = sf_path.strip_prefix(&root)
        {
            e.source_file = rel.to_string_lossy().into_owned();
        }
    }

    // Convert to IndexMap for ordered serialisation. The per-item serde
    // conversion is independent and dominates wall time on large corpora,
    // so fan out via Rayon above the per-file threshold.
    let to_indexmap = |v: Value| -> Option<indexmap::IndexMap<String, Value>> {
        if let Value::Object(m) = v {
            Some(m.into_iter().collect())
        } else {
            None
        }
    };
    let mut nodes_out: Vec<indexmap::IndexMap<String, Value>> =
        if all_nodes.len() >= PARALLEL_THRESHOLD {
            all_nodes
                .into_par_iter()
                .filter_map(|n| serde_json::to_value(n).ok().and_then(to_indexmap))
                .collect()
        } else {
            all_nodes
                .into_iter()
                .filter_map(|n| serde_json::to_value(n).ok().and_then(to_indexmap))
                .collect()
        };
    // Tag AST provenance so the incremental watch rebuild can distinguish
    // AST-extracted nodes from semantic/LLM nodes. On a full re-extraction the
    // watcher drops any AST-marked node missing from the fresh output even when
    // its source file still exists (#1116/#1118).
    for n in &mut nodes_out {
        n.insert("_origin".to_string(), Value::String("ast".to_string()));
    }
    let edges_out: Vec<indexmap::IndexMap<String, Value>> = if all_edges.len() >= PARALLEL_THRESHOLD
    {
        all_edges
            .into_par_iter()
            .filter_map(|e| serde_json::to_value(e).ok().and_then(to_indexmap))
            .collect()
    } else {
        all_edges
            .into_iter()
            .filter_map(|e| serde_json::to_value(e).ok().and_then(to_indexmap))
            .collect()
    };

    ExtractOutput {
        nodes: nodes_out,
        edges: edges_out,
        input_tokens: 0,
        output_tokens: 0,
    }
}
