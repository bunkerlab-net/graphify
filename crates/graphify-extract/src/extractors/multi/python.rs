//! Cross-file Python import + package re-export resolution.
#![allow(clippy::case_sensitive_file_extension_comparisons)]

use super::js::js_node_text;
use super::{JsDefaultResolution, PARALLEL_THRESHOLD, relativise_under_root};
use crate::ids::make_id1;
use crate::import_handlers::make_edge;
use crate::types::{Edge, FileResult, Node, RawCall};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// Emit `uses` edges connecting Python classes to the symbols they import from other files.
///
/// Two-pass: first builds a map of (file-qualified-stem → label → nid) and
/// (bare stem → qualified stem); then re-parses each Python file to find
/// `from X import Y` statements and emit edges. Mirrors Python `_resolve_cross_file_imports`.
pub(super) fn resolve_cross_file_python_imports(
    per_file: &[FileResult],
    paths: &[PathBuf],
) -> Vec<Edge> {
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
pub(super) fn resolve_python_reexport_imports(
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

/// Resolve cross-file Python qualified class-method calls (`ClassName.method()`)
/// to the class-qualified method node (#1446).
///
/// The shared cross-file call pass drops every `is_member_call` because a bare
/// method name collides across the corpus and inflates god-nodes. That guard is
/// right for *instance* calls (`obj.method()`) but misses *class-qualified*
/// calls (`ClassName.method()`), where the receiver is an explicitly-named class
/// — an exact, unambiguous reference. Using the receiver captured by the
/// extractor, when it is a capitalized name resolving to exactly one class node
/// that owns the called method, this emits an EXTRACTED `calls` edge. Purely
/// additive, with a single-definition god-node guard. Mirrors Python
/// `_resolve_python_member_calls`; runs after id-disambiguation.
pub(super) fn resolve_python_member_calls(
    all_nodes: &[Node],
    all_edges: &mut Vec<Edge>,
    all_raw_calls: &[RawCall],
) {
    let key = |s: &str| -> String {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase()
    };

    let node_by_id: HashMap<&str, &Node> = all_nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // A class owns methods: it is the source of one or more `method` edges. Index
    // class label -> owning class node ids (len != 1 is the god-node guard), and
    // (class_node_id, method_key) -> method_node_id.
    let mut class_def_nids: HashMap<String, Vec<String>> = HashMap::new();
    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    for e in all_edges.iter() {
        if e.relation != "method" {
            continue;
        }
        if let Some(cnode) = node_by_id.get(e.source.as_str()) {
            class_def_nids
                .entry(key(cnode.label.as_str()))
                .or_default()
                .push(e.source.clone());
        }
        if let Some(tnode) = node_by_id.get(e.target.as_str()) {
            method_index.insert(
                (e.source.clone(), key(tnode.label.as_str())),
                e.target.clone(),
            );
        }
    }
    if class_def_nids.is_empty() {
        return;
    }
    // A class with N methods produced N entries; collapse to a unique set.
    for nids in class_def_nids.values_mut() {
        nids.sort();
        nids.dedup();
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if !rc.is_member_call || rc.callee.is_empty() || rc.caller_nid.is_empty() {
            continue;
        }
        // Only a capitalized receiver is treated as a class reference, so an
        // instance/module (`self`, `obj`, `config`) never collides with a
        // same-spelled class via the case-folding key.
        let Some(receiver) = rc
            .receiver
            .as_deref()
            .filter(|r| r.chars().next().is_some_and(char::is_uppercase))
        else {
            continue;
        };
        let class_nids = match class_def_nids.get(&key(receiver)) {
            Some(nids) if nids.len() == 1 => nids,
            _ => continue, // absent or ambiguous -> god-node guard
        };
        let Some(method_nid) = method_index.get(&(class_nids[0].clone(), key(&rc.callee))) else {
            continue;
        };
        if *method_nid == rc.caller_nid
            || existing_pairs.contains(&(rc.caller_nid.clone(), method_nid.clone()))
        {
            continue;
        }
        existing_pairs.insert((rc.caller_nid.clone(), method_nid.clone()));
        // EXTRACTED: a qualified `ClassName.method()` is an explicit, unambiguous
        // static reference, and the class resolved to exactly one definition that
        // owns the method.
        new_edges.push(Edge {
            external: false,
            source: rc.caller_nid.clone(),
            target: method_nid.clone(),
            relation: "calls".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: rc.source_file.clone(),
            source_location: Some(rc.source_location.clone()),
            weight: 1.0,
            context: Some("call".to_string()),
            confidence_score: Some(1.0),
        });
    }
    all_edges.extend(new_edges);
}
