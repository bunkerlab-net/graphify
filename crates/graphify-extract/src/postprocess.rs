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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Edge;

    fn n(id: &str, label: &str, file_type: &str, source_file: &str) -> Node {
        Node {
            id: id.to_string(),
            label: label.to_string(),
            file_type: file_type.to_string(),
            source_file: source_file.to_string(),
            source_location: None,
        }
    }

    fn e(src: &str, tgt: &str, relation: &str) -> Edge {
        Edge {
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: String::new(),
            source_file: String::new(),
            source_location: None,
            weight: 0.0,
            context: None,
            confidence_score: None,
        }
    }

    #[test]
    fn node_label_key_strips_punctuation_and_lowercases() {
        assert_eq!(node_label_key("Foo.Bar!"), "foobar");
        assert_eq!(node_label_key("  Hello World  "), "helloworld");
    }

    #[test]
    fn is_type_like_definition_rejects_method_signatures() {
        let method = n("m", "Foo()", "code", "a.py");
        assert!(!is_type_like_definition(&method));
    }

    #[test]
    fn is_type_like_definition_rejects_dotted_labels() {
        let dotted = n("d", "a.b.c", "code", "a.py");
        assert!(!is_type_like_definition(&dotted));
    }

    #[test]
    fn is_type_like_definition_accepts_plain_class() {
        let cls = n("c", "Foo", "code", "a.py");
        assert!(is_type_like_definition(&cls));
    }

    #[test]
    fn rewire_unique_stub_collapses_to_single_real() {
        let mut nodes = vec![
            n("stub", "Foo", "code", ""),
            n("real", "Foo", "code", "a.py"),
            n("user", "Bar", "code", "b.py"),
        ];
        let mut edges = vec![e("user", "stub", "inherits")];
        rewire_unique_stub_nodes(&mut nodes, &mut edges);
        // stub node should be dropped, edge should now target `real`.
        assert!(nodes.iter().all(|n| n.id != "stub"));
        assert_eq!(edges[0].target, "real");
    }

    #[test]
    fn rewire_skips_when_multiple_real_definitions_match() {
        let mut nodes = vec![
            n("stub", "Foo", "code", ""),
            n("real_a", "Foo", "code", "a.py"),
            n("real_b", "Foo", "code", "b.py"),
        ];
        let mut edges = vec![e("user", "stub", "inherits")];
        rewire_unique_stub_nodes(&mut nodes, &mut edges);
        // ambiguous — stub should remain, edge untouched.
        assert!(nodes.iter().any(|n| n.id == "stub"));
        assert_eq!(edges[0].target, "stub");
    }
}
