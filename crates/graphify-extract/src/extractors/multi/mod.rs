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
mod cpp;
mod csharp;
mod java;
mod js;
mod objc;
mod pascal_resolution;
mod python;
mod resolvers;
mod ruby;
mod swift;
mod typescript;

use crate::extractors::{
    clear_xaml_csharp_class_cache, extract_apex, extract_astro, extract_bash, extract_blade,
    extract_c, extract_cpp, extract_csharp, extract_csproj, extract_dart, extract_delphi_form,
    extract_dm, extract_dmf, extract_dmi, extract_dmm, extract_elixir, extract_fortran, extract_go,
    extract_groovy, extract_java, extract_js, extract_json, extract_julia, extract_kotlin,
    extract_lazarus_form, extract_lazarus_package, extract_lua, extract_markdown,
    extract_mcp_config, extract_objc, extract_package_manifest, extract_pascal, extract_php,
    extract_powershell, extract_powershell_manifest, extract_python, extract_razor, extract_ruby,
    extract_rust, extract_scala, extract_sln, extract_slnx, extract_sql, extract_svelte,
    extract_swift, extract_terraform, extract_verilog, extract_vue, extract_xaml, extract_zig,
    is_mcp_config_path, with_xaml_extract_root,
};
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, ExtractOutput, FileResult, Node, RawCall};
use cache::extract_single_file;
use csharp::{resolve_cross_file_csharp_imports, resolve_csharp_type_references};
use java::{resolve_cross_file_java_imports, resolve_java_type_references};
use js::{resolve_js_default_imports, resolve_js_reexport_imports};
use python::{resolve_cross_file_python_imports, resolve_python_reexport_imports};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// JS/TS/JSX source extensions whose modules have no implicit cross-module scope:
/// a direct cross-file call from one is real only with import evidence (#1659).
const JS_TS_CALL_SUFFIXES: [&str; 8] =
    [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"];

const PARALLEL_THRESHOLD: usize = 20;

// ── Dispatch table ────────────────────────────────────────────────────────────

type ExtractFn = fn(&Path) -> FileResult;

/// Strong Objective-C indicators. The four `@` directives are illegal in C/C++,
/// so any of them is a near-zero-false-positive Objective-C signal; `#import` is
/// a Clang/GCC extension technically legal in C/C++ but overwhelmingly Objective-C in
/// practice (real C/C++ uses `#include`). Finding any in a `.h`/`.m` file marks
/// it Objective-C (`.h` routing #1475; `.m`-vs-MATLAB routing #1702). `@property`
/// is excluded: it doubles as a Doxygen command and only ever appears inside an
/// @interface/@protocol anyway, which the stronger directives already cover.
/// Mirrors Python `_OBJC_HEADER_MARKERS`.
const OBJC_HEADER_MARKERS: [&str; 5] = [
    "@interface",
    "@protocol",
    "@implementation",
    "@import",
    "#import",
];

/// Whether a `.h`/`.m` file is Objective-C (vs C/C++ or MATLAB) (#1475, #1702). Sniffs the
/// first 256 KiB for an ObjC-only directive; like Python `_is_objc_header` but
/// reads only the inspected prefix rather than loading a whole (possibly huge,
/// generated) header into memory.
fn is_objc_header(path: &Path) -> bool {
    let Some(head) = read_header_head(path) else {
        return false;
    };
    let text = String::from_utf8_lossy(&head);
    OBJC_HEADER_MARKERS.iter().any(|m| text.contains(m))
}

/// C++-only signals: none are valid in a plain C header, so finding one in a
/// `.h` is a high-confidence signal it is C++ (#1547). The C grammar has no
/// `class_specifier`, so a C++ class header routed to `extract_c` loses the class
/// and its method prototypes; `extract_cpp` recovers it. Conservative — a plain C
/// header matches nothing here. Objective-C sniffing keeps priority (Objective-C++ may
/// legitimately contain `::`/`class` inline). Mirrors Python `_CPP_HEADER_MARKERS`.
const CPP_HEADER_MARKERS: [&str; 7] = [
    "class ",
    "namespace ",
    "template",
    "::",
    "public:",
    "private:",
    "protected:",
];

/// Whether a `.h` file is C++ rather than plain C (#1547). Mirrors
/// `_is_cpp_header`: used only to reroute a non-ObjC `.h` from `extract_c` to
/// `extract_cpp`. Conservative by construction.
fn is_cpp_header(path: &Path) -> bool {
    let Some(head) = read_header_head(path) else {
        return false;
    };
    let text = String::from_utf8_lossy(&head);
    CPP_HEADER_MARKERS.iter().any(|m| text.contains(m))
}

/// Read up to the first 256 KiB of `path` for a header-marker sniff, or `None`
/// on I/O error. Bounds memory for huge generated headers.
fn read_header_head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.take(256 * 1024).read_to_end(&mut head).ok()?;
    Some(head)
}

/// Return the per-language extractor function for a given file path, or `None` for unknown types.
///
/// Blade templates are identified by the `.blade.php` suffix before the extension is checked, so
/// that `foo.blade.php` routes to `extract_blade` rather than `extract_php`. All other languages
/// are dispatched solely on the file extension.
fn get_extractor(path: &Path) -> Option<ExtractFn> {
    // Blade templates: checked by suffix before extension (case-insensitive #1671).
    let name = path.file_name().map_or("", |n| n.to_str().unwrap_or(""));
    if name.to_ascii_lowercase().ends_with(".blade.php") {
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
    let raw_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    // #1671: prefer the exact-case extension (so Fortran's `.F`/`.f` case — a
    // preprocessing signal re-read inside extract_fortran — is preserved), else
    // fall back to its lowercase so capitalized / mixed-case extensions like
    // `.PY`/`.H`/`.TS` are not silently skipped. Mirrors Python
    // `suffix not in _DISPATCH and suffix.lower() in _DISPATCH`.
    let lower_ext = raw_ext.to_ascii_lowercase();
    let ext = if dispatch_ext(raw_ext).is_some() {
        raw_ext
    } else if raw_ext != lower_ext && dispatch_ext(&lower_ext).is_some() {
        lower_ext.as_str()
    } else {
        raw_ext
    };
    // `.h` is C/C++/ObjC-ambiguous; the extension map below sends `.h` to
    // extract_c, which can't read `@interface`/`class`. ObjC sniffing has
    // priority — an Objective-C++ header can carry both `@interface` and inline
    // C++ (`::`) and must parse as ObjC. Then a C++ class header (the C grammar
    // has no class_specifier) reroutes to extract_cpp (#1547). Mirrors Python.
    if ext == "h" {
        if is_objc_header(path) {
            return Some(extract_objc);
        }
        if is_cpp_header(path) {
            return Some(extract_cpp);
        }
    }
    // `.m` is Objective-C OR MATLAB/Octave. The extension map routes `.m` to
    // extract_objc, which force-parses MATLAB through the ObjC grammar into
    // garbage nodes/edges — worse than skipping (#1702). Route to extract_objc
    // only when the file carries an ObjC directive; otherwise leave it without an
    // extractor (surfaced by the no-AST-extractor warning). `.mm` is unambiguously
    // Objective-C++ and is not sniffed. Mirrors Python `_is_objc_source`.
    if ext == "m" && !is_objc_header(path) {
        return None;
    }
    // Extensionless executables (CLI entry points like `devctl`/`manage`) carry
    // their language in the shebang, not the suffix. `classify_file` already
    // routes them to the code path via `shebang_interpreter`; honor the same
    // signal here or they are labeled code and then silently contribute nothing.
    // Only interpreters with a real extractor map; detect's wider set (perl, fish,
    // tcsh, Rscript) stays unmapped and skipped. Mirrors Python `_SHEBANG_DISPATCH`.
    if ext.is_empty()
        && let Some(interp) = graphify_detect::shebang_interpreter(path)
    {
        return shebang_extractor(&interp);
    }
    dispatch_ext(ext)
}

/// Whether `path` has an AST extractor (any supported source language).
///
/// Mirrors graphify-py `_get_extractor(path) is not None`. Used by watch's
/// stale-source reconciliation to distinguish AST-extractable sources — which
/// may be evicted when their file is gone — from documents/data with no
/// extractor (e.g. `.r`), whose nodes must be preserved.
#[must_use]
pub fn has_extractor(path: &Path) -> bool {
    get_extractor(path).is_some()
}

/// Map a shebang interpreter basename to its extractor, or `None` when no real
/// extractor exists (so the file is skipped rather than mis-parsed). Mirrors
/// Python `_SHEBANG_DISPATCH`.
fn shebang_extractor(interp: &str) -> Option<ExtractFn> {
    match interp {
        "python" | "python2" | "python3" => Some(extract_python),
        "bash" | "sh" | "dash" | "zsh" | "ksh" => Some(extract_bash),
        "node" | "nodejs" => Some(extract_js),
        "ruby" => Some(extract_ruby),
        "lua" => Some(extract_lua),
        "php" => Some(extract_php),
        "julia" => Some(extract_julia),
        _ => None,
    }
}

/// Map an (already case-normalized) file extension to its extractor. Split from
/// [`get_extractor`] so the case-insensitive fallback (#1671) can probe key
/// membership via `dispatch_ext(...).is_some()`.
fn dispatch_ext(ext: &str) -> Option<ExtractFn> {
    match ext {
        "py" => Some(extract_python),
        "js" | "jsx" | "mjs" | "ts" | "tsx" | "mts" | "cts" => Some(extract_js),
        "vue" => Some(extract_vue),
        "go" => Some(extract_go),
        "rs" => Some(extract_rust),
        "java" => Some(extract_java),
        "groovy" | "gradle" => Some(extract_groovy),
        "c" | "h" => Some(extract_c),
        "cpp" | "cc" | "cxx" | "hpp" | "cu" | "cuh" | "metal" => Some(extract_cpp),
        "rb" | "rake" => Some(extract_ruby),
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
        "xaml" => Some(extract_xaml),
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

/// A portable `source_file` for a target OUTSIDE the scan root (#1899).
///
/// Produces a walk-up relative path (`../..`-style), degrading to the bare
/// basename when the target lives well outside the corpus (more than three
/// walk-ups, or a different Windows drive) — otherwise its ancestor dirs would
/// embed foreign, possibly user-named segments into a committed graph.json.
/// Mirrors Python `_portable_out_of_root_sf`.
fn portable_out_of_root_sf(p: &Path, root: &Path) -> String {
    let basename = || {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let Some(rel) = lexical_relpath(p, root) else {
        return basename(); // different Windows drive: no relative path exists
    };
    let updepth = rel.split('/').take_while(|seg| *seg == "..").count();
    if updepth > 3 { basename() } else { rel }
}

/// Lexical relative path from `base` to `target` (both absolute), using `..`
/// walk-ups. Returns `None` when they share no common root (e.g. different
/// Windows drive prefixes), mirroring Python `os.path.relpath` raising.
fn lexical_relpath(target: &Path, base: &Path) -> Option<String> {
    use std::path::Component;
    let keep = |c: &Component<'_>| !matches!(c, Component::CurDir);
    let t: Vec<Component> = target.components().filter(keep).collect();
    let b: Vec<Component> = base.components().filter(keep).collect();
    // No shared root/prefix → no relative path exists.
    if t.first() != b.first() {
        return None;
    }
    let mut i = 0;
    while i < t.len() && i < b.len() && t[i] == b[i] {
        i += 1;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in i..b.len() {
        parts.push("..".to_string());
    }
    for c in &t[i..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        return Some(".".to_string());
    }
    Some(parts.join("/"))
}

/// `true` when `id` begins with `prefix` followed by a `_` segment boundary —
/// i.e. `id == "{prefix}_{suffix}"`. IDs are `make_id` output (lowercase word
/// chars + `_`), so the byte index is always on a char boundary.
fn prefix_segment_match(id: &str, prefix: &str) -> bool {
    id.len() > prefix.len() && id.starts_with(prefix) && id.as_bytes()[prefix.len()] == b'_'
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
    // Mirror Python `extract()`'s `_XAML_CSHARP_CLASS_CACHE.clear()` so a repeated
    // in-process run re-scans `.cs` ViewModels instead of serving stale members.
    clear_xaml_csharp_class_cache();

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

    // #1774: the cache is an OUTPUT, so with no explicit `cache_root` it lands
    // under the current working directory — never `root` (the inferred common
    // parent of the inputs), which would drop `graphify-out/` inside a
    // read-only/foreign corpus. `root` still anchors content-hash keys, node
    // ids, symbol resolution, and the XAML boundary; only the cache directory's
    // location diverges from it.
    let cache_location: PathBuf = {
        let base = cache_root.map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            Path::to_path_buf,
        );
        base.canonicalize().unwrap_or(base)
    };

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
            .map(|(idx, path)| (*idx, extract_single_file(path, &root, &cache_location)))
            .collect();
        for (idx, result) in results {
            per_file[idx] = result;
        }
    } else {
        // Sequential
        for (idx, path) in &uncached_work {
            per_file[*idx] = extract_single_file(path, &root, &cache_location);
        }
    }

    // #1689: a file counted as code (extension in CODE_EXTENSIONS) but with no AST
    // extractor wired up (e.g. `.r`/`.R`) silently contributes zero nodes. Surface
    // it, grouped by extension, rather than reporting success as if mapped.
    let mut no_extractor: HashMap<String, usize> = HashMap::new();
    for p in paths {
        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            let ext_l = ext.to_ascii_lowercase();
            if graphify_detect::CODE_EXTENSIONS.contains(&ext_l.as_str())
                && get_extractor(p).is_none()
            {
                *no_extractor.entry(ext_l).or_insert(0) += 1;
            }
        }
    }
    if !no_extractor.is_empty() {
        let mut items: Vec<(String, usize)> = no_extractor.into_iter().collect();
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let total: usize = items.iter().map(|(_, n)| *n).sum();
        let by_count = items
            .iter()
            .map(|(ext, n)| format!(".{ext} ({n})"))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "  warning: {total} file(s) are classified as code but graphify has no AST \
             extractor for their language, so they contributed nothing to the graph: \
             {by_count}. Please open an issue to request support for these (#1689)."
        );
    }

    // #1666: surface a source file an extractor accepted but that produced zero
    // nodes (not even a file node) — silently absent from the graph, blinding
    // affected/explain to it. A rerun retries it (empties are no longer cached).
    let empty_sources: Vec<&PathBuf> = paths
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            per_file[*i].nodes.is_empty()
                && per_file[*i].error.is_none()
                && get_extractor(p).is_some()
        })
        .map(|(_, p)| p)
        .collect();
    if !empty_sources.is_empty() {
        let shown = empty_sources
            .iter()
            .take(5)
            .map(|p| {
                p.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>()
            .join(", ");
        let more = if empty_sources.len() > 5 {
            format!(" (+{} more)", empty_sources.len() - 5)
        } else {
            String::new()
        };
        eprintln!(
            "  warning: {} source file(s) produced zero nodes and are absent from the \
             graph: {shown}{more}. A re-run will retry them (empties are no longer \
             cached); if it persists, please report the file(s) (#1666).",
            empty_sources.len()
        );
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

    // Canonical C# namespace nodes (#1562) intentionally share one digest id
    // across every file that declares the namespace, so the per-file walk emits a
    // duplicate per file. Collapse each id to a single node, keeping the one with
    // the smallest (source_file, source_location) so the survivor is deterministic
    // regardless of input file order (mirrors graphify-py's sorted canonicaliser).
    // The shared id means every file→namespace `contains` edge already points at
    // the survivor, so no edge remap is needed.
    {
        let mut chosen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let key = |n: &Node| {
            (
                n.source_file.clone(),
                n.source_location.clone().unwrap_or_default(),
            )
        };
        for (i, n) in all_nodes.iter().enumerate() {
            if n.node_type.as_deref() != Some("namespace") {
                continue;
            }
            match chosen.get(&n.id) {
                Some(&j) if key(&all_nodes[j]) <= key(n) => {}
                _ => {
                    chosen.insert(n.id.clone(), i);
                }
            }
        }
        let keep: std::collections::HashSet<usize> = chosen.into_values().collect();
        let mut idx = 0usize;
        all_nodes.retain(|n| {
            let this = idx;
            idx += 1;
            n.node_type.as_deref() != Some("namespace") || keep.contains(&this)
        });
    }
    all_edges.extend(cross_edges);

    // Merge a header-declared class (and its methods) with its sibling-impl
    // definition into ONE node (C/C++/ObjC #1547/#1556). Runs BEFORE the id remap
    // below: a header symbol and its impl counterpart share an id only while both
    // still carry the raw file-stem prefix; the per-file prefix remap then diverges
    // them (foo_h vs foo_cpp), so the collapse must happen first. Collapsing here
    // also means disambiguation sees one source_file per id and won't split them.
    crate::postprocess::merge_decl_def_classes(&mut all_nodes, &mut all_edges);

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
    let mut prefix_remap: HashMap<String, Vec<(String, String)>> = HashMap::new();
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
        // Each file maps from up to TWO old prefixes: the input-form
        // `file_node_id(path)` and the absolute-resolved-form
        // `file_node_id(canonicalize(path))`. Alias/workspace imports resolve
        // specifiers through the canonical absolute path, so their symbol-edge
        // targets are keyed off the ABSOLUTE file stem; with relative inputs the
        // two forms differ and absolute-derived targets would otherwise orphan
        // (#1529). Stored as a list so the symbol-prefix remap can try both.
        let mut old_prefs: Vec<(String, String)> = Vec::new();
        let old_pref = crate::ids::file_node_id(path);
        if old_pref != new_id {
            old_prefs.push((old_pref.clone(), new_id.clone()));
        }
        if let Ok(canon) = path.canonicalize() {
            let old_pref_abs = crate::ids::file_node_id(&canon);
            if old_pref_abs != new_id && old_pref_abs != old_pref {
                old_prefs.push((old_pref_abs, new_id.clone()));
            }
        }
        if !old_prefs.is_empty() {
            prefix_remap.insert(path.to_string_lossy().into_owned(), old_prefs);
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
        // A module-level indirect callback's raw_call carries the FILE node as its
        // caller_nid; the file-id remap above rewrites that node, so rewrite the
        // raw_call too or the cross-file `indirect_call` edge dangles on a stale
        // file id (its target-side canonical file node changed, #1565).
        for rc in &mut all_raw_calls {
            if let Some(new_id) = id_remap.get(&rc.caller_nid) {
                rc.caller_nid = new_id.clone();
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
            let Some(entry) = prefix_remap.get(&n.source_file) else {
                continue;
            };
            // The node may carry the input-form OR the absolute-form prefix
            // (#1529); try each, first match wins (source_file gating above
            // prevents cross-file contamination).
            let mut matched = false;
            for (old_pref, new_pref) in entry {
                if prefix_segment_match(&n.id, old_pref) {
                    let new_nid = format!("{new_pref}{}", &n.id[old_pref.len()..]);
                    if new_nid != n.id {
                        sym_remap.insert(n.id.clone(), new_nid);
                    }
                    matched = true;
                    break;
                }
            }
            // When the node is already canonical, also map its absolute-form id
            // variant → the canonical id, so an alias/workspace import edge that
            // targeted the absolute prefix (its file resolved via the canonical
            // path) reconnects instead of dangling (#1529). The absolute prefix
            // is a full-path stem, so it cannot collide with a canonical id.
            if !matched {
                for (old_pref, new_pref) in entry {
                    if old_pref != new_pref && prefix_segment_match(&n.id, new_pref) {
                        let abs_id = format!("{old_pref}{}", &n.id[new_pref.len()..]);
                        sym_remap.entry(abs_id).or_insert_with(|| n.id.clone());
                    }
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

    // Cross-file C# type-reference + import resolution. First re-point dangling
    // inherits/implements/references edges left on shadow stubs, disambiguating
    // same-named types by each file's `using` directives + enclosing namespace
    // (#1562); then re-point resolvable `using` import edges to their canonical
    // namespace / type nodes (#1552). Order mirrors graphify-py: type references
    // harvest the import edges' alias/using metadata, so import cleanup runs last.
    let cs_type_paths: Vec<PathBuf> = paths
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "cs"))
        .cloned()
        .collect();
    if !cs_type_paths.is_empty() {
        resolve_csharp_type_references(&cs_type_paths, &mut all_nodes, &mut all_edges);
        resolve_cross_file_csharp_imports(&mut all_nodes, &mut all_edges);
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
    // Build label → [nid] (skip rationale). Index by EXACT case: case is semantic
    // in most languages, so folding collapses `Path` (class) into `PATH` (env var)
    // and makes a single shell variable the #1 god-node (#1581). Only genuinely
    // case-insensitive languages (PHP/SQL/Nim) also get a folded key so their
    // legitimate case-insensitive resolution still works.
    let mut global_label_to_nids: HashMap<String, Vec<String>> = HashMap::new(); // exact-case
    let mut global_label_to_nids_ci: HashMap<String, Vec<String>> = HashMap::new(); // ci-lang nodes
    let mut nid_to_source_file: HashMap<&str, &str> = HashMap::new();
    for n in &all_nodes {
        if !n.source_file.is_empty() {
            nid_to_source_file.insert(n.id.as_str(), n.source_file.as_str());
        }
        if n.file_type == "rationale" {
            continue;
        }
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        if !normalised.is_empty() {
            global_label_to_nids
                .entry(normalised.to_string())
                .or_default()
                .push(n.id.clone());
            if crate::lang_configs::lang_is_case_insensitive(&n.source_file) {
                global_label_to_nids_ci
                    .entry(normalised.to_lowercase())
                    .or_default()
                    .push(n.id.clone());
            }
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

    // A node and its file node share the exact same `source_file` string, and the
    // file node is the one whose label is the path basename (`add_node(file_nid,
    // path.name)`). Resolving file membership by that shared string is robust
    // against the path-resolution/symlink mismatch that makes the relativised
    // derivation fall back to a non-matching absolute-derived id — which would
    // spuriously fail import evidence and (with the #1659 JS/TS gate below) drop a
    // legitimately-imported call.
    let mut sf_to_file_nid: HashMap<&str, &str> = HashMap::new();
    for n in &all_nodes {
        if n.source_file.is_empty() {
            continue;
        }
        let basename = Path::new(&n.source_file)
            .file_name()
            .and_then(|s| s.to_str());
        if basename == Some(n.label.as_str()) {
            sf_to_file_nid
                .entry(n.source_file.as_str())
                .or_insert(n.id.as_str());
        }
    }

    // Map node → file node id. Prefer the shared-`source_file` lookup; fall back to
    // deriving it from the relativised path (same as `id_remap`, including the
    // canonicalise fallback for symlinked absolute paths).
    let mut nid_to_file_nid: HashMap<String, String> = HashMap::new();
    for n in &all_nodes {
        if n.source_file.is_empty() {
            continue;
        }
        if let Some(&fnid) = sf_to_file_nid.get(n.source_file.as_str()) {
            nid_to_file_nid.insert(n.id.clone(), fnid.to_string());
            continue;
        }
        let sf_path = PathBuf::from(&n.source_file);
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

    // Function / method / class def ids, read from the durable `_callable` node
    // marker AFTER id-remap (mirrors graphify-py's `callable_nids`): the cross-file
    // `indirect_call` resolver binds a callback-by-name only to a real callable,
    // never a same-named data symbol (#1565/#1566).
    let callable_nids: std::collections::HashSet<&str> = all_nodes
        .iter()
        .filter(|n| {
            n.metadata.as_ref().is_some_and(|m| {
                m.get("_callable").and_then(serde_json::Value::as_bool) == Some(true)
            })
        })
        .map(|n| n.id.as_str())
        .collect();
    // Call-like pairs only (calls | indirect_call), for the indirect dedup: a
    // benign `imports` edge to the same symbol must NOT suppress an indirect_call
    // (JS/TS named imports create such an edge), so this is narrower than
    // `existing_pairs`. Only a direct call / prior indirect_call pre-empts it.
    let mut call_like_pairs: std::collections::HashSet<(String, String)> = all_edges
        .iter()
        .filter(|e| matches!(e.relation.as_str(), "calls" | "indirect_call"))
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
        // Exact-case match first (case is semantic, #1581): fold only when the
        // CALLING file's language is case-insensitive (PHP/SQL/Nim), against the
        // folded index of case-insensitive-language nodes — so a Python `Path()`
        // call can never resolve to a shell `PATH` node.
        let callee_lower = rc.callee.to_lowercase();
        let caller = &rc.caller_nid;
        // Resolve the caller's file via the raw_call's own `source_file` string,
        // which is stable regardless of any `caller_nid` remap (#1659), falling back
        // to the caller node's file mapping.
        let caller_file_nid: Option<&str> = sf_to_file_nid
            .get(rc.source_file.as_str())
            .copied()
            .or_else(|| nid_to_file_nid.get(caller).map(String::as_str));
        // A renamed default-import binding (`import mk from './foo'; mk()`) aliases
        // the local name to the origin symbol; prefer that over global label
        // matching, since the local name has no node of its own (#6dc23db). The
        // alias is keyed by the EXACT local name — JS/TS is case-sensitive (#1581).
        let alias_tgt = caller_file_nid
            .and_then(|f| js_default_aliases.get(&(f.to_string(), rc.callee.clone())));
        let candidates: Vec<&String> = if let Some(t) = alias_tgt {
            vec![t]
        } else {
            let mut c = global_label_to_nids
                .get(rc.callee.as_str())
                .map_or_else(Vec::new, |v| v.iter().collect());
            if c.is_empty() && crate::lang_configs::lang_is_case_insensitive(&rc.source_file) {
                c = global_label_to_nids_ci
                    .get(&callee_lower)
                    .map_or_else(Vec::new, |v| v.iter().collect());
            }
            c
        };
        // Cross-language guard: never bind a call to a definition in a different
        // language family (bf7fa50). A TSX callback resolved to a same-named Kotlin
        // method, and a bare Python call to a Kotlin `fun` — phantom edges the spec
        // forbids. Candidates whose family is unknown (no source_file, non-code
        // nodes) are kept (previous permissive behaviour); real interop pairs
        // (Kotlin↔Java, C↔C++↔ObjC, JS↔TS) share a family and still resolve. A
        // caller with an unmapped extension skips the guard entirely.
        let candidates: Vec<&String> =
            if let Some(caller_family) = crate::lang_configs::lang_family(&rc.source_file) {
                candidates
                    .into_iter()
                    .filter(|c| {
                        nid_to_source_file
                            .get(c.as_str())
                            .and_then(|sf| crate::lang_configs::lang_family(sf))
                            .is_none_or(|cf| cf == caller_family)
                    })
                    .collect()
            } else {
                candidates
            };
        // Only resolve unambiguous matches
        if candidates.len() != 1 {
            continue;
        }
        let tgt = candidates[0];
        if tgt == caller {
            continue;
        }
        // Cross-file indirect dispatch (#1565/#1566): a callback passed BY NAME
        // (`from .h import fn; pool.submit(fn)`, or listed in a dispatch table).
        // Emitted as a distinct INFERRED `indirect_call` — ONLY when the target is
        // a real callable def — and BEFORE the `existing_pairs`/#1659 gates: a
        // benign `imports` edge must not suppress it, and the ref stays INFERRED
        // regardless of import evidence (a value reference here, not an invocation).
        if rc.indirect {
            let cl_pair = (caller.clone(), tgt.clone());
            if !callable_nids.contains(tgt.as_str()) || call_like_pairs.contains(&cl_pair) {
                continue;
            }
            call_like_pairs.insert(cl_pair);
            all_edges.push(Edge {
                external: false,
                source: caller.clone(),
                target: tgt.clone(),
                relation: "indirect_call".to_string(),
                confidence: "INFERRED".to_string(),
                source_file: rc.source_file.clone(),
                source_location: Some(rc.source_location.clone()),
                weight: 1.0,
                context: Some(rc.context.clone().unwrap_or_else(|| "argument".to_string())),
                confidence_score: Some(0.8),
                deferred: false,
                metadata: None,
            });
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

        // #1659: a JS/TS DIRECT call with no import evidence is almost always an
        // unrelated same-named export in a package that was never imported — a
        // phantom cross-package edge (a 14-package monorepo showed `platform`/
        // `sidecar` depending on `registry-protocol` purely because it exported
        // generically-named symbols). JS/TS modules have no implicit cross-module
        // scope, so leave it unresolved rather than binding by name alone. Other
        // languages keep single-candidate resolution: C/C++ headers, Ruby autoload,
        // and same-package implicit scope legitimately call across files with no
        // explicit import.
        // Match get_extractor's case-insensitive dispatch (#1659): a `.JS`/`.TS`
        // file is still JS/TS, so the phantom-edge guard must catch it too.
        if !has_import_evidence
            && crate::lang_configs::ends_with_suffix_ci(&rc.source_file, &JS_TS_CALL_SUFFIXES)
        {
            continue;
        }

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
            deferred: false,
            metadata: None,
        });
    }

    // Cross-file, language-specific member-call resolution (#1356 Swift, #1446
    // Python, #1499 Ruby). Runs after the shared call pass — node ids and
    // caller_nids are final — and before source_file relativisation (Swift's
    // type-table re-parse keys on the absolute paths nodes/raw_calls still
    // carry). Each pass is suffix-gated and additive; a new language registers
    // one resolver instead of editing this body.
    let resolver_set = resolvers::default_resolvers();
    resolvers::run_language_resolvers(
        paths,
        &all_nodes,
        &mut all_edges,
        &all_raw_calls,
        &resolver_set,
    );

    // Relativise source_file so persisted paths are portable across machines
    // (#555), then drop the internal origin_file hint entirely. The colliding-id
    // pass above already consumed it (#1462); keeping it would ship an absolute,
    // machine-specific path into graph.json — the same "no absolute paths in
    // output" contract that relativises source_file (#1516, #932). The per-file
    // AST cache keeps its own copy, which the colliding-id pass reads on a cache
    // hit.
    // A source_file OUTSIDE the scan root (an out-of-root ProjectReference/.sln
    // project, or bash `source`) can't be made relative to root; leaving it
    // absolute leaked the scan path — including the OS username — into a
    // committed graph.json (#1899). Fall back to a portable walk-up (or bare
    // basename when the target is far outside), and when a node's id was itself
    // minted from the absolute path, remap it to a portable `ext_`-namespaced id.
    let mut ext_id_remap: HashMap<String, String> = HashMap::new();
    for n in &mut all_nodes {
        let sf_path = PathBuf::from(&n.source_file);
        if sf_path.is_absolute() {
            if let Some(rel) = relativise_under_root(&sf_path, &root) {
                n.source_file = rel.to_string_lossy().into_owned();
            } else {
                let portable = portable_out_of_root_sf(&sf_path, &root);
                if n.id == make_id1(&sf_path.to_string_lossy()) {
                    ext_id_remap.insert(n.id.clone(), make_id(&["ext", &portable]));
                }
                n.source_file = portable;
            }
        }
        n.origin_file = None;
        // Drop the internal `_callable` marker — it rides the AST cache + id-remap
        // for the cross-file indirect resolver but must never ship to graph.json
        // (mirrors graphify-py's `n.pop("_callable")`). Empty the map back to None
        // so a node that carried only this marker serialises identically.
        if let Some(m) = n.metadata.as_mut() {
            m.shift_remove("_callable");
            if m.is_empty() {
                n.metadata = None;
            }
        }
    }
    for e in &mut all_edges {
        let sf_path = PathBuf::from(&e.source_file);
        if sf_path.is_absolute() {
            if let Some(rel) = relativise_under_root(&sf_path, &root) {
                e.source_file = rel.to_string_lossy().into_owned();
            } else {
                e.source_file = portable_out_of_root_sf(&sf_path, &root);
            }
        }
    }
    // Apply the id remap so edge endpoints follow the portable node ids (#1899).
    if !ext_id_remap.is_empty() {
        for n in &mut all_nodes {
            if let Some(new_id) = ext_id_remap.get(&n.id) {
                n.id = new_id.clone();
            }
        }
        for e in &mut all_edges {
            if let Some(new_id) = ext_id_remap.get(&e.source) {
                e.source = new_id.clone();
            }
            if let Some(new_id) = ext_id_remap.get(&e.target) {
                e.target = new_id.clone();
            }
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
    // AST-extracted nodes/edges from semantic/LLM ones. On a full re-extraction
    // the watcher drops any AST-marked node missing from the fresh output even
    // when its source file still exists (#1116/#1118); edges carry the same
    // marker so edge eviction is tier-scoped — re-extracting a source replaces
    // its AST edges without evicting the semantic edges the AST pass cannot
    // regenerate (#1865).
    for n in &mut nodes_out {
        n.insert("_origin".to_string(), Value::String("ast".to_string()));
    }
    let mut edges_out: Vec<indexmap::IndexMap<String, Value>> =
        if all_edges.len() >= PARALLEL_THRESHOLD {
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
    for e in &mut edges_out {
        e.insert("_origin".to_string(), Value::String("ast".to_string()));
    }

    ExtractOutput {
        nodes: nodes_out,
        edges: edges_out,
        input_tokens: 0,
        output_tokens: 0,
    }
}
