//! Community splitting passes: size-based and low-cohesion re-splitting.

use indexmap::IndexMap;

use graphify_build::Graph;

use crate::cohesion::cohesion_score;
use crate::constants::{COHESION_SPLIT_MIN_SIZE, COHESION_SPLIT_THRESHOLD};
use crate::edge_list::{run_partition, subgraph_edge_list};

/// Run a second Louvain pass on a community subgraph to split it
/// further.
///
/// Returns a list of node-lists; each inner list is one sub-community.
/// If the subgraph has no edges the nodes are returned as individual
/// singletons. If the second pass yields only one community the original
/// list is returned unchanged (no artificial split).
pub(crate) fn split_community(graph: &Graph, nodes: &[String]) -> Vec<Vec<String>> {
    let (sub_nodes, sub_edges) = subgraph_edge_list(graph, nodes);

    if sub_edges.is_empty() {
        let mut singletons: Vec<String> = sub_nodes;
        singletons.sort_unstable();
        return singletons.into_iter().map(|n| vec![n]).collect();
    }

    let partition = run_partition(&sub_nodes, &sub_edges, 1.0);

    let mut sub_communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    for (node, cid) in partition {
        sub_communities.entry(cid).or_default().push(node);
    }

    if sub_communities.len() <= 1 {
        let mut sorted = nodes.to_vec();
        sorted.sort_unstable();
        return vec![sorted];
    }

    sub_communities
        .into_values()
        .map(|mut v| {
            v.sort_unstable();
            v
        })
        .collect()
}

/// Apply oversized-community splitting followed by low-cohesion
/// re-splitting.
///
/// First pass: any community with more than `max_size` nodes is split via
/// a second Louvain pass. Second pass: communities with at least
/// [`COHESION_SPLIT_MIN_SIZE`] nodes and cohesion below
/// [`COHESION_SPLIT_THRESHOLD`] are also split.
pub(crate) fn apply_splits(
    communities: Vec<Vec<String>>,
    graph: &Graph,
    max_size: usize,
) -> Vec<Vec<String>> {
    let mut after_size: Vec<Vec<String>> = Vec::new();
    for nodes in communities {
        if nodes.len() > max_size {
            after_size.extend(split_community(graph, &nodes));
        } else {
            after_size.push(nodes);
        }
    }

    let mut after_cohesion: Vec<Vec<String>> = Vec::new();
    for nodes in after_size {
        if nodes.len() >= COHESION_SPLIT_MIN_SIZE
            && cohesion_score(graph, &nodes) < COHESION_SPLIT_THRESHOLD
        {
            let splits = split_community(graph, &nodes);
            if splits.len() > 1 {
                after_cohesion.extend(splits);
            } else {
                after_cohesion.push(nodes);
            }
        } else {
            after_cohesion.push(nodes);
        }
    }

    after_cohesion
}
