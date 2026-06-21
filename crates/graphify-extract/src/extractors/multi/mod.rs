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

#![allow(clippy::case_sensitive_file_extension_comparisons)]

mod cache;
mod java;
mod js;
mod python;
mod swift;

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
use crate::types::{Edge, ExtractOutput, FileResult, Node, RawCall};
use cache::extract_single_file;
use java::{resolve_cross_file_java_imports, resolve_java_type_references};
use js::{resolve_js_default_imports, resolve_js_reexport_imports};
use python::{resolve_cross_file_python_imports, resolve_python_reexport_imports};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use swift::resolve_swift_member_calls;

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
            && let Some(rel) = relativise_under_root(&sf_path, &root)
        {
            n.source_file = rel.to_string_lossy().into_owned();
        }
    }
    for e in &mut all_edges {
        let sf_path = PathBuf::from(&e.source_file);
        if sf_path.is_absolute()
            && let Some(rel) = relativise_under_root(&sf_path, &root)
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
