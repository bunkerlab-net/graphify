//! Shared edge/label assertion helpers for the graphify-extract integration
//! tests, used by both `parity_semantic_types.rs` and `coverage_collectors.rs`.
//!
//! Files under `tests/common/` are compiled as a shared module rather than a
//! standalone test binary. Not every test binary uses every helper, so
//! dead-code is allowed here.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use graphify_extract::FileResult;

/// Mirror Python `_normalize_symbol_label`: strip wrapping `()` and a leading `.`.
#[must_use]
pub fn normalize_symbol_label(label: &str) -> String {
    label
        .trim_matches(|c| c == '(' || c == ')')
        .trim_start_matches('.')
        .to_string()
}

/// Mirror Python `_edge_labels`: the set of `(source_label, target_label)` pairs
/// for `relation` (optionally filtered by `context`), using normalized labels.
#[must_use]
pub fn edge_labels(
    result: &FileResult,
    relation: &str,
    context: Option<&str>,
) -> HashSet<(String, String)> {
    let labels: HashMap<&str, String> = result
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), normalize_symbol_label(&n.label)))
        .collect();
    let mut pairs: HashSet<(String, String)> = HashSet::new();
    for e in &result.edges {
        if e.relation != relation {
            continue;
        }
        if let Some(ctx) = context
            && e.context.as_deref() != Some(ctx)
        {
            continue;
        }
        let s = labels
            .get(e.source.as_str())
            .cloned()
            .unwrap_or_else(|| e.source.clone());
        let t = labels
            .get(e.target.as_str())
            .cloned()
            .unwrap_or_else(|| e.target.clone());
        pairs.insert((s, t));
    }
    pairs
}

/// `true` if `(src, tgt)` appears among `relation`/`context` edges.
#[must_use]
pub fn has_edge(
    result: &FileResult,
    relation: &str,
    context: Option<&str>,
    src: &str,
    tgt: &str,
) -> bool {
    edge_labels(result, relation, context).contains(&(src.to_string(), tgt.to_string()))
}

/// All node labels, in node order.
#[must_use]
pub fn labels(result: &FileResult) -> Vec<String> {
    result.nodes.iter().map(|n| n.label.clone()).collect()
}

/// The set of distinct edge relations present.
#[must_use]
pub fn relations(result: &FileResult) -> HashSet<String> {
    result.edges.iter().map(|e| e.relation.clone()).collect()
}
