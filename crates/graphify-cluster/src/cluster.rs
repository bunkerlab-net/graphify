//! Top-level [`cluster`] entry point: run Louvain on a graph, post-process
//! the partition, and return the canonical `{community_id → [node_ids]}`
//! mapping.

use indexmap::{IndexMap, IndexSet};

use graphify_build::{Graph, GraphKind};

use crate::constants::{MAX_COMMUNITY_FRACTION, MIN_SPLIT_SIZE};
use crate::edge_list::{run_partition, to_undirected_edge_list};
use crate::hubs::{compute_hub_nodes, reattach_hubs};
use crate::splits::apply_splits;

/// Run Louvain community detection on `graph`.
///
/// Returns `{community_id: [node_ids]}` sorted by community size
/// descending (community 0 is largest). Nodes within each community are
/// sorted alphabetically.
///
/// Behaviour:
/// - Empty graph → empty map.
/// - `DiGraph` is converted to undirected internally.
/// - Graph with no edges → each node is its own community.
/// - Oversized communities (>25% of graph nodes, min 10) are split via a
///   second Louvain pass.
/// - Low-cohesion communities (cohesion < 0.05, size ≥ 50) are
///   re-split.
/// - If `exclude_hubs_percentile` is set (0–100), nodes whose degree
///   exceeds that percentile are excluded from the initial partition
///   and re-attached via majority-neighbour vote.
///
/// # Note on Leiden
///
/// The Python reference uses Leiden (graspologic) and falls back to
/// Louvain. This crate implements only Louvain; Leiden is skipped. See
/// `.claude/local/notes/module_cluster.md` for rationale.
#[must_use]
pub fn cluster(
    graph: &Graph,
    resolution: f64,
    exclude_hubs_percentile: Option<f64>,
) -> IndexMap<i64, Vec<String>> {
    if graph.node_count() == 0 {
        return IndexMap::new();
    }

    let undirected_graph: std::borrow::Cow<Graph>;
    let g: &Graph = if graph.kind.is_directed() {
        let mut ug = graph.clone();
        ug.kind = GraphKind::Graph;
        undirected_graph = std::borrow::Cow::Owned(ug);
        &undirected_graph
    } else {
        graph
    };

    let (all_nodes, all_edges) = to_undirected_edge_list(g);

    if g.edge_count() == 0 {
        let mut sorted_nodes: Vec<String> = all_nodes;
        sorted_nodes.sort_unstable();
        return sorted_nodes
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                #[allow(clippy::cast_possible_wrap)] // node index bounded by node count
                (i as i64, vec![n])
            })
            .collect();
    }

    let mut degree_map: IndexMap<String, usize> = IndexMap::new();
    for n in &all_nodes {
        degree_map.insert(n.clone(), 0);
    }
    for (u, v, _) in &all_edges {
        *degree_map.entry(u.clone()).or_insert(0) += 1;
        *degree_map.entry(v.clone()).or_insert(0) += 1;
    }

    let hub_nodes: IndexSet<String> = exclude_hubs_percentile
        .map(|pct| compute_hub_nodes(&degree_map, pct))
        .unwrap_or_default();

    let isolates: Vec<String> = all_nodes
        .iter()
        .filter(|n| degree_map.get(*n).copied().unwrap_or(0) == 0 && !hub_nodes.contains(*n))
        .cloned()
        .collect();

    let connected_nodes: Vec<String> = all_nodes
        .iter()
        .filter(|n| degree_map.get(*n).copied().unwrap_or(0) > 0 && !hub_nodes.contains(*n))
        .cloned()
        .collect();

    let connected_set: IndexSet<&str> = connected_nodes.iter().map(String::as_str).collect();
    let connected_edges: Vec<(String, String, f64)> = all_edges
        .iter()
        .filter(|(u, v, _)| {
            connected_set.contains(u.as_str()) && connected_set.contains(v.as_str())
        })
        .cloned()
        .collect();

    let mut raw: IndexMap<i64, Vec<String>> = IndexMap::new();
    if !connected_nodes.is_empty() {
        let partition = run_partition(&connected_nodes, &connected_edges, resolution);
        for (node, cid) in partition {
            raw.entry(cid).or_default().push(node);
        }
    }

    let mut next_cid: i64 = raw.keys().copied().max().map_or(0, |m| m + 1);
    for node in isolates {
        raw.insert(next_cid, vec![node]);
        next_cid += 1;
    }

    if !hub_nodes.is_empty() {
        reattach_hubs(hub_nodes, &all_edges, &mut raw, &mut next_cid);
    }

    #[allow(clippy::cast_precision_loss)] // node count bounded; precision loss acceptable
    #[allow(clippy::cast_possible_truncation)] // intentional floor for max size
    #[allow(clippy::cast_sign_loss)] // MAX_COMMUNITY_FRACTION is positive constant
    let max_size = MIN_SPLIT_SIZE.max((g.node_count() as f64 * MAX_COMMUNITY_FRACTION) as usize);

    let mut final_communities = apply_splits(raw.into_values().collect(), g, max_size);

    final_communities.sort_unstable_by_key(|v| std::cmp::Reverse(v.len()));

    final_communities
        .into_iter()
        .enumerate()
        .map(|(i, mut nodes)| {
            nodes.sort_unstable();
            #[allow(clippy::cast_possible_wrap)] // community index bounded by node count
            (i as i64, nodes)
        })
        .collect()
}
