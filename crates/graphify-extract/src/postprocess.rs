//! Post-processing passes that run after per-file extraction has been
//! flattened into a single graph.
//!
//! Ports the new helpers added in `graphify-py/graphify/extract.py`
//! (`_disambiguate_colliding_node_ids`, `_rewire_unique_stub_nodes`).
//! These were factored out of the Python `extract()` driver so each
//! corpus-level fix-up step can be unit-tested in isolation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::ids::make_id;
use crate::types::{Edge, Node, RawCall};
use serde_json::Value;

#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static NON_ALNUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9]+").expect("static label-key regex"));

/// Header file suffixes (without the dot): a C/ObjC/C++ quoted include always
/// targets the header, so an import edge dangling on a salted-away bare id is
/// repointed to the header variant of the colliding id (#1475).
const HEADER_SUFFIXES: [&str; 4] = ["h", "hpp", "hh", "hxx"];

/// C-family source/header suffixes (without the dot). Only an importer whose own
/// file is C-family emits `#include` edges that should resolve to a header
/// variant; restricting the header remap to these importers stops a non-C
/// `imports_from` edge whose target merely collides with a header id from being
/// silently mis-pointed at the header (#1475). graphify-py omits this guard.
const C_FAMILY_SUFFIXES: [&str; 11] = [
    "c", "cc", "cpp", "cxx", "c++", "h", "hpp", "hh", "hxx", "m", "mm",
];

/// First 6 hex chars of the SHA-1 of `s` — an injective-enough salt to split
/// node ids whose naive disambiguator still collides (#1522). Matches Python's
/// `hashlib.sha1(...).hexdigest()[:6]`.
fn sha1_hex6(s: &str) -> String {
    use sha1::{Digest, Sha1};
    hex::encode(Sha1::digest(s.as_bytes()))[..6].to_string()
}

/// Canonical form of `source_file` used for disambiguating colliding
/// node IDs. Mirrors `_source_key` in the Python source.
#[must_use]
pub fn source_key(source_file: &str, root: &Path) -> String {
    if source_file.is_empty() {
        return String::new();
    }
    let path = PathBuf::from(source_file);
    if let Ok(canonical) = path.canonicalize()
        && let Ok(rel) = canonical.strip_prefix(root)
    {
        return rel.to_string_lossy().into_owned();
    }
    path.to_string_lossy().into_owned()
}

/// Disambiguation source key for a node: its `source_file`, or its `origin_file`
/// when sourceless (a cross-file reference stub). Mirrors Python
/// `_node_disambiguation_source_key` (#1462) — same-label stubs from different
/// referencing files split into distinct ids, while a real definition (which
/// carries a `source_file`) can still be rewired onto a sourceless stub.
#[must_use]
fn node_disambiguation_source_key(node: &Node, root: &Path) -> String {
    if node.source_file.is_empty() {
        source_key(node.origin_file.as_deref().unwrap_or_default(), root)
    } else {
        source_key(&node.source_file, root)
    }
}

/// Salt every node id in one collision group (`old_id` shared across distinct
/// source files) with its source path, recording `(old_id, source_key) -> new_id`
/// in `remap` and rewriting the node ids. When the naive salt
/// `make_id(source_key, old_id)` itself collides (separator-vs-punctuation paths,
/// #1522), a short sha1 of the raw source path is appended so the colliders split.
fn salt_collision_group(
    old_id: &str,
    group: &[usize],
    source_keys: &HashSet<String>,
    nodes: &mut [Node],
    root: &Path,
    taken: &mut HashSet<String>,
    remap: &mut HashMap<(String, String), String>,
) {
    let mut naive: HashMap<String, String> = HashMap::new();
    for sk in source_keys {
        if !sk.is_empty() {
            naive.insert(sk.clone(), make_id(&[sk, old_id]));
        }
    }
    let mut naive_counts: HashMap<&str, usize> = HashMap::new();
    for nid in naive.values() {
        *naive_counts.entry(nid.as_str()).or_default() += 1;
    }
    let needs_hash: HashSet<String> = naive
        .iter()
        .filter(|(_, nid)| naive_counts.get(nid.as_str()).copied().unwrap_or(0) > 1)
        .map(|(sk, _)| sk.clone())
        .collect();
    for &idx in group {
        let sk = node_disambiguation_source_key(&nodes[idx], root);
        if sk.is_empty() {
            continue;
        }
        // Same-file same-id nodes share a `(old_id, sk)` key and must collapse to
        // one disambiguated id; if a prior node in this group already minted it,
        // reuse it rather than minting (and bumping) a second id, which would
        // split them and corrupt the remap.
        let new_id = if let Some(existing) = remap.get(&(old_id.to_string(), sk.clone())) {
            existing.clone()
        } else {
            let naive_id = naive
                .get(&sk)
                .cloned()
                .unwrap_or_else(|| make_id(&[&sk, old_id]));
            // Divergence from graphify-py (#1522): the reference only de-dupes
            // within the group. Hash when the naive id collides in-group or with
            // an id already claimed — a surviving non-ambiguous id (a salted
            // `src_a_foo` can clash with a real node already named that) or one
            // minted earlier in this pass. `taken` is seeded with surviving ids
            // (never an ambiguous id about to be rewritten), so this never
            // over-hashes.
            let mut candidate = if needs_hash.contains(&sk) || taken.contains(&naive_id) {
                make_id(&[&sk, old_id, &sha1_hex6(&sk)])
            } else {
                naive_id
            };
            // If the hashed candidate is also taken, widen with a numeric suffix
            // until globally unique (terminates: `taken` is finite).
            let mut bump = 1u32;
            while taken.contains(&candidate) {
                candidate = make_id(&[&sk, old_id, &sha1_hex6(&sk), &bump.to_string()]);
                bump += 1;
            }
            taken.insert(candidate.clone());
            candidate
        };
        remap.insert((old_id.to_string(), sk), new_id.clone());
        if new_id != *old_id {
            nodes[idx].id = new_id;
        }
    }
}

/// Build `old_id -> header-variant new_id` for colliding ids whose group includes
/// exactly one header file (`.h`/`.hpp`/…), so a quoted-include import edge
/// dangling on the salted-away bare id is repointed to the header variant
/// (#1475). Divergence from graphify-py: the reference picks the *first* header
/// in node order, which is arbitrary when a group holds two same-stem headers
/// (e.g. `foo.h` and `foo.hpp`); we only remap when the header target is
/// unambiguous and otherwise leave the edge to the normal per-source-file remap.
fn build_header_remaps(
    ambiguous_ids: &HashSet<String>,
    by_id: &HashMap<String, Vec<usize>>,
    nodes: &[Node],
    root: &Path,
    remap: &HashMap<(String, String), String>,
) -> HashMap<String, String> {
    let mut header_remaps: HashMap<String, String> = HashMap::new();
    for old_id in ambiguous_ids {
        let Some(group) = by_id.get(old_id) else {
            continue;
        };
        let mut header_ids: HashSet<String> = HashSet::new();
        for &idx in group {
            let sk = node_disambiguation_source_key(&nodes[idx], root);
            let is_header = Path::new(&sk)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| HEADER_SUFFIXES.contains(&e.to_lowercase().as_str()));
            if !sk.is_empty()
                && is_header
                && let Some(new_id) = remap.get(&(old_id.clone(), sk))
            {
                header_ids.insert(new_id.clone());
            }
        }
        // Only remap when exactly one header variant exists; multiple headers
        // make the import target ambiguous, so we leave it untouched.
        if header_ids.len() == 1
            && let Some(new_id) = header_ids.into_iter().next()
        {
            header_remaps.insert(old_id.clone(), new_id);
        }
    }
    header_remaps
}

/// Rewrite only node IDs that collide across two or more *distinct*
/// source files, using the source path as the disambiguator.
///
/// Two `Program.cs` files in different directories produce identical
/// `make_id("Program")` IDs by default. This pass detects the collision
/// and rewrites each colliding node's ID to `make_id(source_key, old_id)`.
/// Edges and raw calls are rewritten via a per-source-key remap so they
/// continue to point at the right (newly-qualified) node.
///
/// Mirrors `_disambiguate_colliding_node_ids` in the Python source.
pub fn disambiguate_colliding_node_ids(
    nodes: &mut [Node],
    edges: &mut [Edge],
    raw_calls: &mut [RawCall],
    root: &Path,
) {
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, node) in nodes.iter().enumerate() {
        // Module anchor nodes (#1327) intentionally share one id across every
        // file importing the same module; disambiguating them by source path
        // would scatter a single module into N file-qualified duplicates.
        if node
            .metadata
            .as_ref()
            .and_then(|m| m.get("type"))
            .and_then(Value::as_str)
            == Some("module")
        {
            continue;
        }
        // Canonical C# namespace nodes (#1562) likewise share one digest id
        // across the files that declare the namespace; they are already
        // deduplicated to a single node upstream, but exempt them here too so a
        // future duplicate can never be scattered into file-qualified ids.
        if node.node_type.as_deref() == Some("namespace") {
            continue;
        }
        if !node.id.is_empty() {
            by_id.entry(node.id.clone()).or_default().push(idx);
        }
    }

    let mut ambiguous_ids: HashSet<String> = HashSet::new();
    for (old_id, group) in &by_id {
        let source_keys: HashSet<String> = group
            .iter()
            .map(|&idx| node_disambiguation_source_key(&nodes[idx], root))
            .collect();
        if group.len() >= 2 && source_keys.len() >= 2 {
            ambiguous_ids.insert(old_id.clone());
        }
    }

    // Ids already claimed, which a salted id must avoid: every surviving id plus
    // every id minted during this pass. A non-ambiguous id always survives; an
    // ambiguous id survives only when one of its nodes has an empty disambiguation
    // source key (`salt_collision_group` skips those, leaving the bare id intact).
    // Seeded before salting, so it never holds an ambiguous id about to be
    // rewritten (which would needlessly over-hash); `salt_collision_group` adds
    // each minted id so a later group can't reuse an earlier group's salted form
    // (possible when two old ids normalise to the same salted id).
    let mut taken: HashSet<String> = by_id
        .iter()
        .filter(|(id, group)| {
            !ambiguous_ids.contains(*id)
                || group
                    .iter()
                    .any(|&idx| node_disambiguation_source_key(&nodes[idx], root).is_empty())
        })
        .map(|(id, _)| id.clone())
        .collect();

    let mut remap: HashMap<(String, String), String> = HashMap::new();
    // Iterate in sorted order so the `taken`-set resolution is deterministic
    // regardless of `ambiguous_ids` hash order.
    let mut ambiguous_sorted: Vec<&String> = ambiguous_ids.iter().collect();
    ambiguous_sorted.sort();
    for old_id in ambiguous_sorted {
        let Some(group) = by_id.get(old_id) else {
            continue;
        };
        let source_keys: HashSet<String> = group
            .iter()
            .map(|&idx| node_disambiguation_source_key(&nodes[idx], root))
            .collect();
        salt_collision_group(
            old_id,
            group,
            &source_keys,
            nodes,
            root,
            &mut taken,
            &mut remap,
        );
    }
    if remap.is_empty() {
        return;
    }

    // Some non-colliding nodes already had their ID rewritten by an
    // earlier pipeline stage (e.g. the file-node id remap). Mirror the
    // Python "single-unique-candidate" remap so edges referencing the
    // old ID still resolve.
    let mut unambiguous_remaps: HashMap<String, String> = HashMap::new();
    for (old_id, group) in &by_id {
        if ambiguous_ids.contains(old_id) {
            continue;
        }
        let candidates: HashSet<String> = group
            .iter()
            .filter_map(|&idx| {
                let new_id = &nodes[idx].id;
                if new_id == old_id {
                    None
                } else {
                    Some(new_id.clone())
                }
            })
            .collect();
        if candidates.len() == 1
            && let Some(new_id) = candidates.into_iter().next()
        {
            unambiguous_remaps.insert(old_id.clone(), new_id);
        }
    }

    let header_remaps = build_header_remaps(&ambiguous_ids, &by_id, nodes, root, &remap);

    rewrite_edge_endpoints(edges, &remap, &unambiguous_remaps, &header_remaps, root);

    for call in raw_calls.iter_mut() {
        let call_source_key = source_key(&call.source_file, root);
        let caller_tuple = (call.caller_nid.clone(), call_source_key);
        if let Some(new_id) = remap.get(&caller_tuple) {
            call.caller_nid.clone_from(new_id);
        } else if let Some(new_id) = unambiguous_remaps.get(&call.caller_nid) {
            call.caller_nid.clone_from(new_id);
        }
    }
}

/// Rewrite edge endpoints onto disambiguated node ids. Source endpoints take the
/// per-source-file salt remap then the single-candidate remap; target endpoints
/// additionally resolve a C-family `#include` to the header variant first (#1475).
fn rewrite_edge_endpoints(
    edges: &mut [Edge],
    remap: &HashMap<(String, String), String>,
    unambiguous_remaps: &HashMap<String, String>,
    header_remaps: &HashMap<String, String>,
    root: &Path,
) {
    for edge in edges.iter_mut() {
        let edge_source_key = source_key(&edge.source_file, root);
        let source_key_tuple = (edge.source.clone(), edge_source_key.clone());
        let target_key_tuple = (edge.target.clone(), edge_source_key);
        if let Some(new_id) = remap.get(&source_key_tuple) {
            edge.source.clone_from(new_id);
        } else if let Some(new_id) = unambiguous_remaps.get(&edge.source) {
            edge.source.clone_from(new_id);
        }
        // A C-family `#include "foo.h"` whose bare id was salted away resolves to
        // the header variant BEFORE the same-source-file salt is considered, so a
        // `.m` including its own `.h` points at the header, not back at itself.
        // Restrict to C-family importers: a non-C `imports_from` whose target
        // merely collides with a header id must NOT be rewritten to the header
        // (#1475). graphify-py applies this to every imports/imports_from edge and
        // can mis-target non-C imports — fixed here per the parity-bug rule.
        let importer_is_c_family = Path::new(&edge.source_file)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| C_FAMILY_SUFFIXES.contains(&e.to_lowercase().as_str()));
        if importer_is_c_family
            && matches!(edge.relation.as_str(), "imports" | "imports_from")
            && let Some(new_id) = header_remaps.get(&edge.target)
        {
            edge.target.clone_from(new_id);
        } else if let Some(new_id) = remap.get(&target_key_tuple) {
            edge.target.clone_from(new_id);
        } else if let Some(new_id) = unambiguous_remaps.get(&edge.target) {
            edge.target.clone_from(new_id);
        }
    }
}

/// Map any unresolved no-source-file stub node to a unique real
/// definition with the same label.
///
/// Cross-language inheritance edges (e.g. a C# class inheriting from a
/// Python class) emit a placeholder stub node with no `source_file`.
/// When exactly one type-like definition with the same label exists in
/// the corpus, the stub is dropped and every edge endpoint pointing at
/// it is redirected to the real node.
///
/// Mirrors `_rewire_unique_stub_nodes` in the Python source.
pub fn rewire_unique_stub_nodes(nodes: &mut Vec<Node>, edges: &mut [Edge]) {
    let mut real_by_label: HashMap<String, Vec<usize>> = HashMap::new(); // exact-case (all langs)
    let mut real_by_label_ci: HashMap<String, Vec<usize>> = HashMap::new(); // case-INSENSITIVE-lang reals only
    let mut stub_indices: Vec<usize> = Vec::new();

    for (idx, node) in nodes.iter().enumerate() {
        let key = node_label_key(&node.label, false);
        if key.is_empty() {
            continue;
        }
        if !node.source_file.is_empty() {
            if is_type_like_definition(node) {
                // Match stubs case-SENSITIVELY: a `Path` reference must not rewire to a
                // `PATH` env var (#1581). Fold only for genuinely case-insensitive
                // languages, where `foo` legitimately resolves to `Foo`.
                real_by_label.entry(key).or_default().push(idx);
                if crate::lang_configs::lang_is_case_insensitive(&node.source_file) {
                    real_by_label_ci
                        .entry(node_label_key(&node.label, true))
                        .or_default()
                        .push(idx);
                }
            }
            continue;
        }
        stub_indices.push(idx);
    }

    let mut remap: HashMap<String, String> = HashMap::new();
    let mut drop_ids: HashSet<String> = HashSet::new();
    for &stub_idx in &stub_indices {
        let stub = &nodes[stub_idx];
        if stub.id.is_empty() {
            continue;
        }
        let mut candidates = real_by_label
            .get(&node_label_key(&stub.label, false))
            .cloned()
            .unwrap_or_default();
        if candidates.len() != 1 {
            // No unique exact match — fall back to a case-insensitive match, but only
            // against case-insensitive-language definitions (so a case-sensitive `PATH`
            // can never absorb a `Path` reference).
            candidates = real_by_label_ci
                .get(&node_label_key(&stub.label, true))
                .cloned()
                .unwrap_or_default();
            if candidates.len() != 1 {
                continue;
            }
        }
        let target_id = nodes[candidates[0]].id.clone();
        if !target_id.is_empty() && target_id != stub.id {
            remap.insert(stub.id.clone(), target_id);
            drop_ids.insert(stub.id.clone());
        }
    }

    if remap.is_empty() {
        return;
    }

    for edge in edges.iter_mut() {
        if let Some(new_id) = remap.get(&edge.source) {
            edge.source.clone_from(new_id);
        }
        if let Some(new_id) = remap.get(&edge.target) {
            edge.target.clone_from(new_id);
        }
    }

    nodes.retain(|n| !drop_ids.contains(&n.id));
}

fn node_label_key(label: &str, fold: bool) -> String {
    let trimmed = label.trim();
    let key = NON_ALNUM.replace_all(trimmed, "");
    if fold {
        key.to_lowercase()
    } else {
        key.into_owned()
    }
}

fn is_type_like_definition(node: &Node) -> bool {
    let label = node.label.trim();
    if label.is_empty() {
        return false;
    }
    if label.ends_with(')') || label.starts_with('.') {
        return false;
    }
    if label.contains('.') {
        return false;
    }
    node.file_type == "code"
}

/// Collapse cross-file Swift `extension Foo` nodes into the canonical
/// `Foo` declaration.
///
/// tree-sitter-swift reuses `class_declaration` for both `class Foo` and
/// `extension Foo`, and node IDs carry the file stem, so each file that
/// extends `Foo` produces its own `Foo` node. This pass re-parses each
/// `.swift` file to identify which class nodes were actually `extension`
/// declarations, then matches them by label against the corpus's
/// non-extension nodes. When exactly one match exists the extension's
/// edges are remapped onto the canonical node and the extension node is
/// dropped. Extensions of types outside the corpus, and ambiguous
/// labels, are left untouched.
///
/// Mirrors `_merge_swift_extensions` in graphify-py `extract.py`.
pub fn merge_swift_extensions(paths: &[PathBuf], nodes: &mut Vec<Node>, edges: &mut Vec<Edge>) {
    // Re-parse each Swift file to collect the type names declared as
    // `extension Foo`, keyed by the file path string the extractor recorded in
    // `source_file`. Re-parsing once here is cheaper than threading a sidecar
    // through the generic walker.
    let mut ext_names_by_file: HashMap<String, HashSet<String>> = HashMap::new();

    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .is_err()
    {
        return;
    }

    for path in paths {
        if path.extension().is_none_or(|e| e != "swift") {
            continue;
        }
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut names: HashSet<String> = HashSet::new();
        collect_swift_extension_names(tree.root_node(), &source, &mut names);
        if !names.is_empty() {
            ext_names_by_file.insert(path.to_string_lossy().into_owned(), names);
        }
    }

    if ext_names_by_file.is_empty() {
        return;
    }

    // Identify the actual extension nodes by (source_file, label) rather than a
    // re-derived id — the file-node-id remap (#1033/#1096) rewrites symbol ids,
    // so matching on the id we'd compute from the path no longer holds.
    let mut extension_nids: HashSet<String> = HashSet::new();
    let mut extension_labels: HashMap<String, String> = HashMap::new();
    for node in nodes.iter() {
        if ext_names_by_file
            .get(&node.source_file)
            .is_some_and(|names| names.contains(&node.label))
        {
            extension_nids.insert(node.id.clone());
            extension_labels.insert(node.id.clone(), node.label.clone());
        }
    }

    if extension_nids.is_empty() {
        return;
    }

    // Build label → [canonical_nid] from corpus nodes (excluding the
    // extension nodes themselves).
    let mut label_to_canonical: HashMap<String, Vec<String>> = HashMap::new();
    for node in nodes.iter() {
        if extension_nids.contains(&node.id) {
            continue;
        }
        if node.label.is_empty() {
            continue;
        }
        label_to_canonical
            .entry(node.label.clone())
            .or_default()
            .push(node.id.clone());
    }

    let mut remap: HashMap<String, String> = HashMap::new();
    for ext_nid in &extension_nids {
        let Some(label) = extension_labels.get(ext_nid) else {
            continue;
        };
        let candidates = label_to_canonical.get(label).cloned().unwrap_or_default();
        if candidates.len() != 1 {
            continue;
        }
        if candidates[0] != *ext_nid {
            remap.insert(ext_nid.clone(), candidates[0].clone());
        }
    }

    if remap.is_empty() {
        return;
    }

    nodes.retain(|n| !remap.contains_key(&n.id));

    // Rewrite edges, drop self-loops created by the merge, and dedup on
    // (src, tgt, relation, source_file, source_location).
    let mut rewritten: Vec<Edge> = Vec::with_capacity(edges.len());
    let mut seen_keys: HashSet<(String, String, String, String, String)> = HashSet::new();
    for edge in edges.drain(..) {
        let mut edge = edge;
        if let Some(new_src) = remap.get(&edge.source) {
            edge.source.clone_from(new_src);
        }
        if let Some(new_tgt) = remap.get(&edge.target) {
            edge.target.clone_from(new_tgt);
        }
        if edge.source == edge.target {
            continue;
        }
        let key = (
            edge.source.clone(),
            edge.target.clone(),
            edge.relation.clone(),
            edge.source_file.clone(),
            edge.source_location.clone().unwrap_or_default(),
        );
        if seen_keys.contains(&key) {
            continue;
        }
        seen_keys.insert(key);
        rewritten.push(edge);
    }
    *edges = rewritten;
}

/// Implementation file-extension pairing for the decl/def class merge.
const DECLDEF_IMPL_SUFFIXES: [&str; 6] = ["m", "mm", "cpp", "cc", "cxx", "c"];

/// `(dir, base_stem)` for a header/impl source file, else `None`. The base stem
/// strips an Objective-C category suffix (`Foo+Cat.m` -> `Foo`) so a category impl pairs
/// with its `Foo.h` declaration. Files whose extension is neither a header nor an
/// impl extension return `None` and are never merged. Mirrors
/// `_decldef_class_stem`.
fn decldef_class_stem(source_file: &str) -> Option<(String, String)> {
    if source_file.is_empty() {
        return None;
    }
    let p = Path::new(source_file);
    let suffix = p.extension()?.to_string_lossy().to_lowercase();
    if !HEADER_SUFFIXES.contains(&suffix.as_str())
        && !DECLDEF_IMPL_SUFFIXES.contains(&suffix.as_str())
    {
        return None;
    }
    let stem_full = p.file_stem()?.to_string_lossy().into_owned();
    let stem = stem_full.split('+').next().unwrap_or_default().to_string();
    if stem.is_empty() {
        return None;
    }
    let dir = p.parent().map_or_else(String::new, |d| {
        let s = d.to_string_lossy();
        if s.is_empty() {
            ".".to_string()
        } else {
            s.into_owned()
        }
    });
    Some((dir, stem))
}

/// `true` when a source file carries a header extension.
fn is_decldef_header(source_file: &str) -> bool {
    Path::new(source_file)
        .extension()
        .is_some_and(|e| HEADER_SUFFIXES.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

/// Merge a class (and its methods) declared in a header with its definition in a
/// sibling impl file into ONE node, for C/C++/ObjC (#1547, #1556).
///
/// A class declared in `Foo.h` (`class Foo` / `@interface Foo`) and defined in
/// the sibling `Foo.cpp` / `Foo.m` (`@implementation Foo`, plus — after the C++
/// qualified-name fix — out-of-class method definitions `Foo::bar`) produces TWO
/// nodes per symbol that share an id (both keyed off the extension-less file
/// stem) and differ only in `source_file`/`label`. Left alone,
/// `disambiguate_colliding_node_ids` SPLITS them by path, tripping every
/// resolver's single-definition god-node guard. This pass runs BEFORE the id
/// remap and collapses each such id-collision onto the header (declaration)
/// variant; because the colliding nodes already share an id, no edge re-pointing
/// is needed — only the redundant duplicate is dropped and now-identical edges
/// de-duplicated. Mirrors graphify-py `_merge_decl_def_classes`.
///
/// GOD-NODE GUARD: the collapse fires ONLY when every node in an id-collision
/// group is from one sibling header/impl family (same dir, same base stem, header
/// paired with impl) AND the group has exactly ONE header. Two same-named classes
/// in different directories never collide on id, so they are never merged.
pub fn merge_decl_def_classes(nodes: &mut Vec<Node>, edges: &mut Vec<Edge>) {
    // Group every code node index by id.
    let mut by_id: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if n.file_type != "code" || n.id.is_empty() || n.source_file.is_empty() {
            continue;
        }
        by_id.entry(n.id.as_str()).or_default().push(i);
    }

    // Per id-collision, keep the (single) header node and drop the rest — but only
    // for a clean single-header sibling family.
    let mut drop_idx: HashSet<usize> = HashSet::new();
    for group in by_id.values() {
        if group.len() < 2 {
            continue;
        }
        let mut sibling_keys: HashSet<(String, String)> = HashSet::new();
        let mut headers: Vec<usize> = Vec::new();
        let mut ok = true;
        for &idx in group {
            let sf = nodes[idx].source_file.as_str();
            let Some(ds) = decldef_class_stem(sf) else {
                ok = false;
                break;
            };
            sibling_keys.insert(ds);
            if is_decldef_header(sf) {
                headers.push(idx);
            }
        }
        if !ok || sibling_keys.len() != 1 || headers.len() != 1 {
            continue;
        }
        let keeper = headers[0];
        for &idx in group {
            if idx != keeper {
                drop_idx.insert(idx);
            }
        }
    }

    if drop_idx.is_empty() {
        return;
    }

    // Drop the redundant duplicate nodes (the surviving header keeps its own
    // label/source_file; edges are unchanged because the id is identical).
    let mut idx = 0usize;
    nodes.retain(|_| {
        let keep = !drop_idx.contains(&idx);
        idx += 1;
        keep
    });

    // De-dup any now-identical edges and drop self-loops the collapse created.
    let mut seen: HashSet<(String, String, String, Option<String>)> = HashSet::new();
    edges.retain(|e| {
        if e.source == e.target {
            return false;
        }
        seen.insert((
            e.source.clone(),
            e.target.clone(),
            e.relation.clone(),
            e.context.clone(),
        ))
    });
}

fn collect_swift_extension_names(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    names: &mut HashSet<String>,
) {
    // tree-sitter `child()` takes a `u32` index while `child_count()` returns
    // `usize`. AST nodes never exceed 2^32 children in practice; truncate
    // explicitly with the cap so clippy doesn't flag the lossy cast.
    let child_count: u32 = u32::try_from(node.child_count()).unwrap_or(u32::MAX);
    if node.kind() == "class_declaration" {
        let is_extension = (0..child_count)
            .filter_map(|i| node.child(i))
            .any(|c| c.kind() == "extension");
        if is_extension {
            // Find the type name child.
            let name = (0..child_count).find_map(|i| {
                let c = node.child(i)?;
                if matches!(c.kind(), "type_identifier" | "user_type" | "identifier") {
                    let raw = std::str::from_utf8(&source[c.start_byte()..c.end_byte()])
                        .ok()?
                        .trim()
                        .to_string();
                    Some(raw)
                } else {
                    None
                }
            });
            if let Some(name) = name
                && !name.is_empty()
            {
                names.insert(name);
            }
        }
    }
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            collect_swift_extension_names(child, source, names);
        }
    }
}

#[cfg(test)]
#[path = "postprocess_tests.rs"]
mod postprocess_tests;
