//! Leiden community detection (Traag, Waltman & van Eck, 2019).
//!
//! Mirrors the primary partitioning path in `graphify-py/graphify/cluster.py`,
//! which calls `graspologic.partition.leiden(stable, random_seed=42)` first
//! and only falls back to `nx.community.louvain_communities` when graspologic
//! is not installed. The Rust port ships Leiden in-process via the
//! `leiden-rs` crate, so the fallback is never needed at runtime; the
//! pure-Rust Louvain implementation in [`crate::louvain`] is preserved as a
//! backup partitioner and for direct unit testing.
//!
//! Why Leiden over Louvain:
//! - Leiden's refinement phase guarantees connected sub-communities; Louvain
//!   does not, which manifests as flip-flop oscillation on graphs with
//!   tightly-coupled bridge nodes (observed on a real-world 16k-node
//!   monorepo where Louvain's Phase 1 hit a 100-pass safety cap).
//! - Leiden's Fast Local Move (FLM) bounds Phase-1 work to O(moves),
//!   eliminating the unbounded `while improved` loop that motivated the
//!   cap in [`crate::louvain`].

use std::collections::HashMap;

use leiden_rs::graph::builder::GraphDataBuilder;
use leiden_rs::leiden::{Leiden, LeidenConfig};

/// Run Leiden on a node/edge list and return `{node_id → community_id}`.
///
/// Inputs and outputs intentionally match [`crate::louvain::partition`]
/// byte-for-byte so the two implementations are drop-in interchangeable
/// behind [`crate::edge_list::run_partition`].
///
/// Behaviour:
/// - Empty node list → empty map.
/// - Edges whose endpoints are unknown to `nodes` are silently dropped.
/// - Self-loops are dropped (Leiden does not need them for modularity).
/// - Determinism: nodes and edges are sorted before being handed to the
///   builder so a given (nodes, edges, resolution) tuple always produces
///   the same partition. `random_seed=42` matches the Python reference.
/// - If the underlying builder rejects the graph (e.g. zero nodes after
///   the empty-input early-return is bypassed) or the Leiden run fails,
///   we return an empty map so the caller's downstream code paths
///   continue to operate on a valid (if degenerate) partition.
#[must_use]
pub fn partition(
    nodes: &[String],
    edges: &[(String, String, f64)],
    resolution: f64,
) -> HashMap<String, usize> {
    if nodes.is_empty() {
        return HashMap::new();
    }

    // Sort nodes for stable index assignment. The original Louvain port did
    // the same — the assignment is observable through community IDs only
    // indirectly, but stability is required for parity tests and for the
    // `cluster::cluster` final sort (which sorts within-community node
    // lists alphabetically anyway).
    let mut sorted_nodes = nodes.to_vec();
    sorted_nodes.sort_unstable();
    let node_to_idx: HashMap<String, usize> = sorted_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i))
        .collect();

    let mut builder = GraphDataBuilder::new(sorted_nodes.len());

    // Sort edges for determinism. Stable ordering of (src, tgt, weight)
    // ensures the Leiden run sees edges in the same order across calls,
    // which combines with the seeded RNG to give reproducible partitions.
    let mut sorted_edges = edges.to_vec();
    sorted_edges.sort_unstable_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });
    for (src, tgt, w) in &sorted_edges {
        if let (Some(&u), Some(&v)) = (node_to_idx.get(src), node_to_idx.get(tgt))
            && u != v
        {
            // Skip self-loops — they contribute zero to modularity and the
            // builder rejects them on some versions of leiden-rs. A
            // failed `add_edge` (out-of-bounds index, etc.) is silently
            // ignored; the resulting partition is still well-defined
            // modulo the missing edge.
            let _ = builder.add_edge(u, v, *w);
        }
    }

    let Ok(graph_data) = builder.build() else {
        return HashMap::new();
    };

    let config = LeidenConfig::builder()
        .resolution(resolution)
        .seed(42)
        .build();

    let Ok(output) = Leiden::new(config).run(&graph_data) else {
        return HashMap::new();
    };

    let membership = output.partition.as_slice();
    sorted_nodes
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, membership[i]))
        .collect()
}
