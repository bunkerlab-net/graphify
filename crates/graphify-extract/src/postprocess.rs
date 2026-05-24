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

#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static NON_ALNUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9]+").expect("static label-key regex"));

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
        if !node.id.is_empty() {
            by_id.entry(node.id.clone()).or_default().push(idx);
        }
    }

    let mut remap: HashMap<(String, String), String> = HashMap::new();
    let mut ambiguous_ids: HashSet<String> = HashSet::new();
    for (old_id, group) in &by_id {
        let source_keys: HashSet<String> = group
            .iter()
            .map(|&idx| source_key(&nodes[idx].source_file, root))
            .collect();
        if group.len() < 2 || source_keys.len() < 2 {
            continue;
        }
        ambiguous_ids.insert(old_id.clone());
        for &idx in group {
            let sk = source_key(&nodes[idx].source_file, root);
            if sk.is_empty() {
                continue;
            }
            let new_id = make_id(&[&sk, old_id]);
            remap.insert((old_id.clone(), sk), new_id.clone());
            if new_id != *old_id {
                nodes[idx].id = new_id;
            }
        }
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

    for edge in edges.iter_mut() {
        let edge_source_key = source_key(&edge.source_file, root);
        let source_key_tuple = (edge.source.clone(), edge_source_key.clone());
        let target_key_tuple = (edge.target.clone(), edge_source_key);
        if let Some(new_id) = remap.get(&source_key_tuple) {
            edge.source.clone_from(new_id);
        } else if let Some(new_id) = unambiguous_remaps.get(&edge.source) {
            edge.source.clone_from(new_id);
        }
        if let Some(new_id) = remap.get(&target_key_tuple) {
            edge.target.clone_from(new_id);
        } else if let Some(new_id) = unambiguous_remaps.get(&edge.target) {
            edge.target.clone_from(new_id);
        }
    }

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
    // Collect (nid, label) for every Swift class_declaration whose body
    // contains the `extension` keyword. Re-parsing each Swift file once
    // here is cheaper than threading a sidecar through the generic walker.
    let mut extension_nids: HashSet<String> = HashSet::new();
    let mut extension_labels: HashMap<String, String> = HashMap::new();

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
        let stem = crate::ids::file_stem(path);
        collect_swift_extensions(
            tree.root_node(),
            &source,
            &stem,
            &mut extension_nids,
            &mut extension_labels,
        );
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

fn collect_swift_extensions(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    stem: &str,
    extension_nids: &mut HashSet<String>,
    extension_labels: &mut std::collections::HashMap<String, String>,
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
                let nid = make_id(&[stem, &name]);
                extension_labels.insert(nid.clone(), name);
                extension_nids.insert(nid);
            }
        }
    }
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            collect_swift_extensions(child, source, stem, extension_nids, extension_labels);
        }
    }
}

#[cfg(test)]
#[path = "postprocess_tests.rs"]
mod postprocess_tests;
