//! Deterministic symbol indexing and conservative cross-file resolution
//! helpers.
//!
//! Ports the Rust-applicable portion of
//! `graphify-py/graphify/symbol_resolution.py` — the label / index /
//! existing-edge-pair helpers used by every cross-file resolver. Python's
//! AST-based `parse_python_import_aliases` and the resolver entry points
//! that depend on it remain on the Python side; the Rust extract pipeline
//! uses its own tree-sitter-driven import facts (see
//! `generic/*.rs`), so the Python AST shim is not needed here.

use std::collections::HashSet;

use crate::types::{Edge, Node, RawCall};

/// Normalise a node label into the key used for call resolution.
///
/// Mirrors `normalise_callable_label` in the Python source.
#[must_use]
pub fn normalise_callable_label(label: &str) -> String {
    label
        .trim()
        .trim_end_matches("()")
        .trim_start_matches('.')
        .to_lowercase()
}

/// `true` when the node is a valid deterministic call target.
///
/// Mirrors `node_is_resolvable_symbol`. Requires `file_type == "code"` and a
/// non-filename, non-empty normalised label.
#[must_use]
pub fn node_is_resolvable_symbol(node: &Node) -> bool {
    if node.file_type != "code" {
        return false;
    }
    let label = node.label.trim();
    if label.is_empty() {
        return false;
    }
    // DIVERGENCE from graphify-py: `.mts`/`.cts` are added here. graphify-py's
    // node_is_resolvable_symbol lists only `.ts`/`.tsx`, so a `.mts`/`.cts` file
    // node's filename label leaks into the callable symbol index unlike every
    // other TS file — fixed per AGENTS.md (reference bugs are not requirements).
    if [
        ".py", ".js", ".ts", ".tsx", ".mts", ".cts", ".java", ".go", ".rs",
    ]
    .iter()
    .any(|suffix| label.ends_with(suffix))
    {
        return false;
    }
    !normalise_callable_label(label).is_empty()
}

/// Build a `normalised_label → [node_id]` index for cross-file lookup.
#[must_use]
pub fn build_label_index(nodes: &[Node]) -> indexmap::IndexMap<String, Vec<String>> {
    let mut index: indexmap::IndexMap<String, Vec<String>> = indexmap::IndexMap::new();
    for node in nodes {
        if !node_is_resolvable_symbol(node) {
            continue;
        }
        if node.id.is_empty() {
            continue;
        }
        let key = normalise_callable_label(&node.label);
        if key.is_empty() {
            continue;
        }
        index.entry(key).or_default().push(node.id.clone());
    }
    index
}

/// All existing `(source, target, relation, context)` edge tuples.
///
/// Including the relation lets the resolver distinguish a semantically
/// new `calls` edge from an existing `contains` edge between the same
/// endpoints (#F5). Including `context` (defaulting to the empty string
/// when absent) further distinguishes `references/parameter_type` from
/// `references/return_type` on the same `(source, target)` pair, so two
/// reference edges with different contexts both survive deduplication —
/// matches the Python change in `_apply_symbol_resolution_facts`
/// (graphify-py @ ab4e542).
#[must_use]
pub fn existing_edge_pairs(edges: &[Edge]) -> HashSet<(String, String, String, String)> {
    let mut tuples: HashSet<(String, String, String, String)> = HashSet::new();
    for edge in edges {
        if !edge.source.is_empty() && !edge.target.is_empty() {
            tuples.insert((
                edge.source.clone(),
                edge.target.clone(),
                edge.relation.clone(),
                edge.context.clone().unwrap_or_default(),
            ));
        }
    }
    tuples
}

/// Collect raw calls from all per-file fragments. Empty `raw_calls` slices
/// are tolerated; non-finite entries cannot occur in the Rust types since
/// `RawCall` is a typed struct rather than a JSON dict.
#[must_use]
pub fn iter_raw_calls<'a>(per_file: impl IntoIterator<Item = &'a Vec<RawCall>>) -> Vec<RawCall> {
    let mut out: Vec<RawCall> = Vec::new();
    for slice in per_file {
        out.extend(slice.iter().cloned());
    }
    out
}

#[cfg(test)]
#[path = "symbol_resolution_tests.rs"]
mod symbol_resolution_tests;
