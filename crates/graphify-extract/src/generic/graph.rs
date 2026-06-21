//! Low-level graph + AST primitives shared across the generic extractor.
//!
//! `add_node` / `add_edge` build the node/edge lists; the AST helpers
//! (`named_children`, `first_child_kind`, `any_child_kind`, `find_body`,
//! `ensure_named_node`) are reused by every per-language submodule.

#![allow(clippy::cast_possible_truncation)]

use super::config::LangConfig;
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, Node as GNode};
use std::collections::HashSet;
use tree_sitter::Node;

/// Insert a new graph node if `nid` has not been seen before.
///
/// The `seen_ids` set is the deduplication gate — a second call with the same
/// `nid` is silently dropped so that multiple structural passes (e.g.
/// file-level node + function-level) cannot produce duplicate node entries.
pub(crate) fn add_node(
    nid: &str,
    label: &str,
    line: u32,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) {
    if seen_ids.insert(nid.to_string()) {
        nodes.push(GNode {
            id: nid.to_string(),
            label: label.to_string(),
            file_type: "code".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            metadata: None,
        });
    }
}

/// Append an edge to the edge list.
///
/// Unlike nodes, edges are not deduplicated here — the caller is responsible
/// for deduplication via `seen_call_pairs` or the final clean pass in
/// [`extract_generic`].
pub(crate) fn add_edge(
    src: &str,
    tgt: &str,
    relation: &str,
    line: u32,
    str_path: &str,
    context: Option<&str>,
    edges: &mut Vec<Edge>,
) {
    edges.push(Edge {
        external: false,
        source: src.to_string(),
        target: tgt.to_string(),
        relation: relation.to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: Some(format!("L{line}")),
        weight: 1.0,
        context: context.map(str::to_string),
        confidence_score: None,
    });
}

// ── Small AST helpers ──────────────────────────────────────────────────────────

/// Collect the named children of `node` into a `Vec`.
#[must_use]
pub(crate) fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().is_named() {
                out.push(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    out
}

/// Return the first child of `node` whose kind is `kind`.
#[must_use]
pub(crate) fn first_child_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().kind() == kind {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// `true` if any child of `node` has the given `kind` (allocation-free).
#[must_use]
pub(crate) fn any_child_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().kind() == kind {
                return true;
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

// ── Body finder ───────────────────────────────────────────────────────────────

/// Locate the body child of a class or function node.
///
/// First tries the grammar's `body` field; falls back to scanning for a child
/// whose kind appears in `config.body_fallback_child_types`. The fallback is
/// needed for languages like Kotlin whose grammar uses `class_body` or
/// `function_body` node types rather than a named field.
#[must_use]
pub(crate) fn find_body<'tree>(node: Node<'tree>, config: &LangConfig) -> Option<Node<'tree>> {
    if let Some(b) = node.child_by_field_name(config.body_field) {
        return Some(b);
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if config.body_fallback_child_types.contains(&child.kind()) {
                return Some(child);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

// ── ensure_named_node ─────────────────────────────────────────────────────────

/// Return the NID for a named entity, creating a placeholder node if needed.
///
/// First checks for a file-qualified ID (`<stem>_<name>`); if already seen,
/// returns that ID. Otherwise ensures the bare-name node exists (creating it
/// when absent) and returns the bare NID. Used for cross-file type references
/// in C# `field_declaration` processing.
pub(crate) fn ensure_named_node(
    name: &str,
    line: u32,
    stem: &str,
    str_path: &str,
    nodes: &mut Vec<GNode>,
    seen_ids: &mut HashSet<String>,
) -> String {
    let nid1 = make_id(&[stem, name]);
    if seen_ids.contains(&nid1) {
        return nid1;
    }
    let nid2 = make_id1(name);
    if !seen_ids.contains(&nid2) {
        add_node(&nid2, name, line, str_path, nodes, seen_ids);
    }
    nid2
}
