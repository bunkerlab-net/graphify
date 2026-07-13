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
            origin_file: None,
            node_type: None,
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
    add_edge_meta(src, tgt, relation, line, str_path, context, None, edges);
}

/// [`add_edge`] carrying optional edge `metadata` (e.g. C# `ref_token` /
/// `qualified` / `ref_qualifier`, #1562).
#[allow(clippy::too_many_arguments)] // edge fields; grouping into a struct would churn every caller
pub(crate) fn add_edge_meta(
    src: &str,
    tgt: &str,
    relation: &str,
    line: u32,
    str_path: &str,
    context: Option<&str>,
    metadata: Option<indexmap::IndexMap<String, serde_json::Value>>,
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
        deferred: false,
        metadata,
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

/// Return the NID for a named entity, creating a SOURCELESS placeholder stub if
/// needed.
///
/// First checks for a file-qualified ID (`<stem>_<name>`); if already seen,
/// returns that ID. Otherwise ensures a bare-name stub exists (creating it when
/// absent) and returns the bare NID. Used for cross-file type references
/// (Java/C#/Kotlin/Scala/Swift inheritance + field types).
///
/// The stub is SOURCELESS (`source_file` empty) so a real project definition
/// carrying a `source_file` can still be rewired onto it (#1402); the
/// referencing file is recorded as `origin_file` purely to disambiguate
/// same-label stubs from different files during id-collision splitting (#1462).
pub(crate) fn ensure_named_node(
    name: &str,
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
    if seen_ids.insert(nid2.clone()) {
        nodes.push(GNode {
            id: nid2.clone(),
            label: name.to_string(),
            file_type: "code".to_string(),
            source_file: String::new(),
            // Parity dispute (CodeRabbit): `Some("")`, NOT `None`. graphify-py
            // emits `"source_location": ""` for these sourceless cross-file stubs
            // (extract.py ensure_named_node), so `None` (skipped on serialize)
            // would drop the field and break byte-identical JSON. The empty string
            // is the sourceless marker (`!= "L1"`); `origin_file` carries provenance.
            source_location: Some(String::new()),
            metadata: None,
            origin_file: Some(str_path.to_string()),
            node_type: None,
        });
    }
    nid2
}
