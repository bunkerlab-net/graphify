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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::Value;

use crate::extractors::{
    extract_astro, extract_bash, extract_blade, extract_c, extract_cpp, extract_csharp,
    extract_csproj, extract_dart, extract_delphi_form, extract_dm, extract_dmf, extract_dmi,
    extract_dmm, extract_elixir, extract_fortran, extract_go, extract_groovy, extract_java,
    extract_js, extract_json, extract_julia, extract_kotlin, extract_lazarus_form,
    extract_lazarus_package, extract_lua, extract_markdown, extract_mcp_config, extract_objc,
    extract_pascal, extract_php, extract_powershell, extract_python, extract_razor, extract_ruby,
    extract_rust, extract_scala, extract_sln, extract_slnx, extract_sql, extract_svelte,
    extract_swift, extract_verilog, extract_zig, is_mcp_config_path,
};
use crate::ids::make_id1;
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
        "ps1" => Some(extract_powershell),
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

/// Extract a single file, returning a cached result when available.
///
/// Looks up the on-disk AST cache first; on a miss, dispatches to the language-specific
/// extractor and writes the result back to the cache. Files with no matching extractor
/// return an empty `FileResult` rather than an error.
fn extract_single_file(path: &Path, effective_root: &Path) -> FileResult {
    // Check cache
    let cached = graphify_cache::load_cached(path, effective_root, "ast");
    if let Some(v) = cached {
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
    if result.error.is_none() {
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

    // Collapse Swift `extension Foo` nodes onto the canonical `class Foo`
    // declaration. Mirrors `_merge_swift_extensions` in graphify-py.
    crate::postprocess::merge_swift_extensions(paths, &mut all_nodes, &mut all_edges);

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
        let candidates: &[String] = global_label_to_nids
            .get(&callee_key)
            .map_or(&[], Vec::as_slice);
        // Only resolve unambiguous matches
        if candidates.len() != 1 {
            continue;
        }
        let tgt = &candidates[0];
        let caller = &rc.caller_nid;
        if tgt == caller {
            continue;
        }
        let pair = (caller.clone(), tgt.clone());
        if existing_pairs.contains(&pair) {
            continue;
        }

        let caller_file_nid = nid_to_file_nid.get(caller);
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
