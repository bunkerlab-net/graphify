//! Cross-file Java import + type-reference resolution.
#![allow(clippy::case_sensitive_file_extension_comparisons)]

use super::PARALLEL_THRESHOLD;
use crate::ids::make_id1;
use crate::lang_configs::ends_with_suffix_ci;
use crate::types::{Edge, FileResult, Node, RawCall};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
                    deferred: false,
                    metadata: None,
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

/// Emit `imports` edges by resolving Java `import` statements across all extracted files.
///
/// Two-pass: first builds a map of (class-name → [nid]) from all capitalised node labels;
/// then re-parses each `.java` file to find `import_declaration` nodes and emit edges.
/// Mirrors Python `_resolve_cross_file_java_imports`.
#[allow(clippy::too_many_lines)]
pub(super) fn resolve_cross_file_java_imports(
    per_file: &[FileResult],
    paths: &[PathBuf],
) -> Vec<Edge> {
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
// edges correctly stay on their stub. `references` (field/parameter/return-type
// uses) is included so a cross-module reference to a same-named class doesn't
// dangle on a sourceless phantom when two packages define the same simple name —
// the node survives with a path-scoped id, but the reference must point at the
// RIGHT one (#1744). Mirrors the C# resolver, whose set already covers it.
const JAVA_REPOINT_RELATIONS: &[&str] =
    &["implements", "inherits", "extends", "imports", "references"];

/// Re-point dangling Java `implements`/`inherits`/`extends`/`imports` edges that
/// bare-name resolution left on sourceless shadow stubs, using each referencing
/// file's `import` statements (then its package) to disambiguate same-named types
/// across packages (#1318). Drops shadow stubs no edge references anymore.
///
/// Mirrors Python `_resolve_java_type_references`. Runs after id-disambiguation
/// and `rewire_unique_stub_nodes` (so it only handles the ambiguous remainder),
/// in the final node-id space; keyed by the absolute `source_file` strings the
/// nodes/edges still carry before the closing relativisation pass.
pub(super) fn resolve_java_type_references(
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
pub(super) fn is_type_like_definition(node: &Node) -> bool {
    let label = node.label.trim();
    !label.is_empty()
        && !label.ends_with(')')
        && !label.starts_with('.')
        && !label.contains('.')
        && node.file_type == "code"
}

// ── Cross-file Java member-call resolution (#1696) ────────────────────────────

/// Normalise a Java type/method label to a comparison key: strip surrounding
/// whitespace, a leading `.`, and a trailing `()`. Case-sensitive (Java types
/// are). Mirrors the inner `key` of graphify-py `_resolve_java_member_calls`.
fn mc_key(label: &str) -> String {
    let s = label.trim();
    let s = s.strip_prefix('.').unwrap_or(s);
    s.strip_suffix("()").unwrap_or(s).to_string()
}

/// Resolve Java member calls (`gw.charge()`) against the receiver's declared
/// type, so a call binds to the ONE owning class's method instead of every
/// same-named method in the corpus (which produced phantom edges and god nodes).
///
/// `this` binds to the caller's enclosing type (exact); an explicit capitalised
/// receiver is itself the type (exact); a typed field/parameter/local resolves
/// via the extractor's method-scoped `receiver_type` (inferred). A missing,
/// ambiguous, or non-unique target is skipped rather than guessed (the same
/// single-owner god-node guard as the Swift/Ruby resolvers). Purely additive:
/// only handles member calls the shared name-based pass deferred. Mirrors
/// graphify-py `_resolve_java_member_calls`.
pub(super) fn resolve_java_member_calls(
    all_nodes: &[Node],
    all_edges: &mut Vec<Edge>,
    all_raw_calls: &[RawCall],
) {
    let node_by_id: HashMap<&str, &Node> = all_nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let contained: HashSet<&str> = all_edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| e.target.as_str())
        .collect();

    // key(label) -> type-definition node ids (source-backed, contained, type-like).
    let mut type_def_nids: HashMap<String, Vec<String>> = HashMap::new();
    for n in all_nodes {
        if !n.source_file.is_empty()
            && contained.contains(n.id.as_str())
            && is_type_like_definition(n)
        {
            type_def_nids
                .entry(mc_key(&n.label))
                .or_default()
                .push(n.id.clone());
        }
    }

    // (owner_type_nid, method_key) -> method ids, and method -> its owner type,
    // both from `method` ownership edges.
    let mut method_index: HashMap<(String, String), HashSet<String>> = HashMap::new();
    let mut enclosing_type: HashMap<String, String> = HashMap::new();
    for e in all_edges.iter() {
        if e.relation != "method" {
            continue;
        }
        let Some(mnode) = node_by_id.get(e.target.as_str()) else {
            continue;
        };
        enclosing_type
            .entry(e.target.clone())
            .or_insert_with(|| e.source.clone());
        method_index
            .entry((e.source.clone(), mc_key(&mnode.label)))
            .or_default()
            .insert(e.target.clone());
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if !ends_with_suffix_ci(&rc.source_file, &[".java"]) || !rc.is_member_call {
            continue;
        }
        let receiver = match rc.receiver.as_deref() {
            Some(r) if !r.is_empty() => r,
            _ => continue,
        };
        if rc.callee.is_empty() || rc.caller_nid.is_empty() {
            continue;
        }
        let caller = rc.caller_nid.as_str();

        let (type_nid, exact): (String, bool) = if receiver == "this" {
            match enclosing_type.get(caller) {
                Some(t) => (t.clone(), true),
                None => continue,
            }
        } else {
            // Extractor-typed receiver -> INFERRED; a bare capitalised receiver is
            // itself the type name -> EXTRACTED. No type -> skip.
            let (type_name, exact) = match rc.receiver_type.as_deref() {
                Some(t) => (t.to_string(), false),
                None if receiver.chars().next().is_some_and(char::is_uppercase) => {
                    (receiver.to_string(), true)
                }
                None => continue,
            };
            match type_def_nids.get(&mc_key(&type_name)) {
                Some(defs) if defs.len() == 1 => (defs[0].clone(), exact),
                _ => continue, // absent or ambiguous -> god-node guard
            }
        };

        let Some(method_nids) = method_index.get(&(type_nid, mc_key(&rc.callee))) else {
            continue;
        };
        if method_nids.len() != 1 {
            continue;
        }
        let Some(method_nid) = method_nids.iter().next() else {
            continue;
        };
        let method_nid = method_nid.clone();
        if method_nid == caller || !existing_pairs.insert((caller.to_string(), method_nid.clone()))
        {
            continue;
        }
        new_edges.push(Edge {
            external: false,
            source: caller.to_string(),
            target: method_nid,
            relation: "calls".to_string(),
            confidence: if exact { "EXTRACTED" } else { "INFERRED" }.to_string(),
            source_file: rc.source_file.clone(),
            source_location: Some(rc.source_location.clone()),
            weight: 1.0,
            context: Some("call".to_string()),
            confidence_score: Some(if exact { 1.0 } else { 0.8 }),
            deferred: false,
            metadata: None,
        });
    }
    all_edges.extend(new_edges);
}
