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
    occupied: &HashSet<String>,
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
        let naive_id = naive
            .get(&sk)
            .cloned()
            .unwrap_or_else(|| make_id(&[&sk, old_id]));
        // Divergence from graphify-py (#1522): the reference only de-dupes within
        // the group. Hash when the naive id collides in-group OR with an id
        // surviving OUTSIDE this group (a salted `src_a_foo` can clash with a real
        // node already named that). `occupied` holds surviving non-ambiguous ids,
        // so this never over-hashes against an ambiguous id about to be rewritten.
        let mut new_id = if needs_hash.contains(&sk) || occupied.contains(&naive_id) {
            make_id(&[&sk, old_id, &sha1_hex6(&sk)])
        } else {
            naive_id
        };
        // If even the hashed candidate is occupied, widen with a numeric suffix
        // until the id is globally unique (terminates: `occupied` is finite).
        let mut bump = 1u32;
        while occupied.contains(&new_id) {
            new_id = make_id(&[&sk, old_id, &sha1_hex6(&sk), &bump.to_string()]);
            bump += 1;
        }
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

    // Ids that survive disambiguation: a salted id must not collide with one of
    // these. A non-ambiguous id always survives; an ambiguous id survives only
    // when one of its nodes has an empty disambiguation source key, since
    // `salt_collision_group` skips those and leaves the bare id intact. Built
    // before salting, so the guard never targets an ambiguous id that is itself
    // fully rewritten (which would cause needless over-hashing).
    let occupied: HashSet<String> = by_id
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
    for old_id in &ambiguous_ids {
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
            &occupied,
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
    let mut real_by_label: HashMap<String, Vec<usize>> = HashMap::new();
    let mut stub_indices: Vec<usize> = Vec::new();

    for (idx, node) in nodes.iter().enumerate() {
        let key = node_label_key(&node.label);
        if key.is_empty() {
            continue;
        }
        if !node.source_file.is_empty() {
            if is_type_like_definition(node) {
                real_by_label.entry(key).or_default().push(idx);
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
        let candidates = real_by_label
            .get(&node_label_key(&stub.label))
            .cloned()
            .unwrap_or_default();
        if candidates.len() != 1 {
            continue;
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

fn node_label_key(label: &str) -> String {
    let trimmed = label.trim();
    NON_ALNUM.replace_all(trimmed, "").to_lowercase()
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
