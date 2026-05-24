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
    if [".py", ".js", ".ts", ".tsx", ".java", ".go", ".rs"]
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

/// All existing `(source, target, relation)` edge triples.
///
/// Including the relation lets the resolver distinguish a semantically
/// new `calls` edge from an existing `contains` edge between the same
/// endpoints (#F5).
#[must_use]
pub fn existing_edge_pairs(edges: &[Edge]) -> HashSet<(String, String, String)> {
    let mut triples: HashSet<(String, String, String)> = HashSet::new();
    for edge in edges {
        if !edge.source.is_empty() && !edge.target.is_empty() {
            triples.insert((
                edge.source.clone(),
                edge.target.clone(),
                edge.relation.clone(),
            ));
        }
    }
    triples
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
mod tests {
    use super::*;

    fn n(id: &str, label: &str, file_type: &str) -> Node {
        Node {
            id: id.to_string(),
            label: label.to_string(),
            file_type: file_type.to_string(),
            source_file: "src.py".to_string(),
            source_location: None,
        }
    }

    #[test]
    fn normalise_strips_parens_and_dot() {
        assert_eq!(normalise_callable_label("Foo()"), "foo");
        assert_eq!(normalise_callable_label(".bar"), "bar");
        assert_eq!(normalise_callable_label("  Baz  "), "baz");
    }

    #[test]
    fn node_is_resolvable_rejects_filename_labels() {
        assert!(!node_is_resolvable_symbol(&n("a", "module.py", "code")));
        assert!(!node_is_resolvable_symbol(&n("b", "thing.ts", "code")));
    }

    #[test]
    fn node_is_resolvable_rejects_non_code_file_type() {
        assert!(!node_is_resolvable_symbol(&n("a", "Foo", "document")));
    }

    #[test]
    fn node_is_resolvable_accepts_plain_code_label() {
        assert!(node_is_resolvable_symbol(&n("a", "Foo", "code")));
    }

    #[test]
    fn build_label_index_groups_by_normalised_label() {
        let nodes = vec![
            n("a", "Foo()", "code"),
            n("b", "Foo", "code"),
            n("c", "Bar", "code"),
        ];
        let index = build_label_index(&nodes);
        assert_eq!(index["foo"], vec!["a".to_string(), "b".to_string()]);
        assert_eq!(index["bar"], vec!["c".to_string()]);
    }

    #[test]
    fn existing_edge_pairs_includes_relation() {
        let edges = vec![
            Edge {
                source: "a".to_string(),
                target: "b".to_string(),
                relation: "contains".to_string(),
                confidence: String::new(),
                source_file: String::new(),
                source_location: None,
                weight: 0.0,
                context: None,
                confidence_score: None,
            },
            Edge {
                source: "a".to_string(),
                target: "b".to_string(),
                relation: "calls".to_string(),
                confidence: String::new(),
                source_file: String::new(),
                source_location: None,
                weight: 0.0,
                context: None,
                confidence_score: None,
            },
        ];
        let pairs = existing_edge_pairs(&edges);
        assert!(pairs.contains(&("a".to_string(), "b".to_string(), "contains".to_string())));
        assert!(pairs.contains(&("a".to_string(), "b".to_string(), "calls".to_string())));
    }
}
