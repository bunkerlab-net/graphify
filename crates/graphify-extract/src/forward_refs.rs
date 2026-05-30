//! Per-file reconciliation of forward-reference placeholder nodes.
//!
//! A type referenced *before* its declaration in the same file makes
//! `ensure_named_node` mint a bare-name placeholder (`make_id1(name)`), because
//! the file-qualified declaration id is not yet registered when the reference
//! is walked. When the declaration is reached later in the single-pass walk it
//! creates a *second*, file-qualified node, leaving the reference edge pointing
//! at the orphaned placeholder rather than the real declaration.
//!
//! This is a duplicate-node bug carried over from graphify-py's single-pass
//! `ensure_named_node` (`extract.py`). The corpus-level `rewire_unique_stub_nodes`
//! does not fix it because that pass only rewires *no-source-file* stubs, while
//! these placeholders carry the referencing file's path.

use std::collections::HashMap;

use crate::ids::make_id1;
use crate::types::{Edge, Node};

/// Merge forward-reference placeholder nodes into their same-file declarations.
///
/// A node is treated as a placeholder when its id equals `make_id1(label)` (the
/// bare-name shape `ensure_named_node` mints). When exactly one *declaration*
/// node — one whose id is file-qualified, i.e. not the bare shape — shares the
/// placeholder's label, every edge endpoint is redirected from the placeholder
/// id to the declaration id and the placeholder node is dropped.
///
/// Genuinely external types keep their bare id (no same-file declaration shares
/// the label), so cross-file references still resolve during the corpus merge.
/// The "exactly one declaration" guard mirrors `rewire_unique_stub_nodes` and
/// avoids ambiguous merges. Because the pass only acts when a bare placeholder
/// and a same-label declaration coexist — which happens solely on forward
/// references — declare-before-use files are left byte-for-byte unchanged.
pub(crate) fn reconcile_forward_refs(nodes: &mut Vec<Node>, edges: &mut [Edge]) {
    // label -> ids of declaration nodes (those NOT shaped like a placeholder).
    let mut decls_by_label: HashMap<String, Vec<String>> = HashMap::new();
    for node in nodes.iter() {
        if node.id != make_id1(&node.label) {
            decls_by_label
                .entry(node.label.clone())
                .or_default()
                .push(node.id.clone());
        }
    }

    // placeholder id -> declaration id, only when the label resolves to exactly
    // one declaration.
    let mut remap: HashMap<String, String> = HashMap::new();
    for node in nodes.iter() {
        if node.id != make_id1(&node.label) {
            continue;
        }
        if let Some(decls) = decls_by_label.get(&node.label)
            && decls.len() == 1
            && decls[0] != node.id
        {
            remap.insert(node.id.clone(), decls[0].clone());
        }
    }

    if remap.is_empty() {
        return;
    }

    for edge in edges.iter_mut() {
        if let Some(new_id) = remap.get(&edge.target) {
            edge.target.clone_from(new_id);
        }
        if let Some(new_id) = remap.get(&edge.source) {
            edge.source.clone_from(new_id);
        }
    }
    nodes.retain(|node| !remap.contains_key(&node.id));
}
