//! Hub-node handling: identify high-degree nodes for exclusion from the
//! initial Louvain pass, then re-attach them via majority-neighbour
//! voting.

use indexmap::{IndexMap, IndexSet};

/// Compute the set of hub nodes to exclude, given a percentile
/// threshold.
///
/// Nodes whose degree is *strictly greater* than the value at the
/// `pct`-th percentile (sorted ascending) are returned. Empty input
/// produces an empty set.
pub(crate) fn compute_hub_nodes(
    degree_map: &IndexMap<String, usize>,
    pct: f64,
) -> IndexSet<String> {
    let mut degrees: Vec<usize> = degree_map.values().copied().collect();
    degrees.sort_unstable();
    if degrees.is_empty() {
        return IndexSet::new();
    }
    // pct is in [0,100]; product is non-negative, floor is intentional
    #[allow(clippy::cast_precision_loss)] // node count bounded; precision loss negligible
    #[allow(clippy::cast_sign_loss)] // pct is in [0,100], product is non-negative
    #[allow(clippy::cast_possible_truncation)] // intentional floor to get index
    let idx = ((degrees.len() as f64 * pct / 100.0) as usize).saturating_sub(1);
    let idx = idx.min(degrees.len() - 1);
    let threshold = degrees[idx];
    degree_map
        .iter()
        .filter(|&(_, &d)| d > threshold)
        .map(|(n, _)| n.clone())
        .collect()
}

/// Re-attach hub nodes to communities via majority-neighbour vote.
///
/// For each hub (sorted alphabetically for determinism), count the
/// neighbours in each existing community and join the winner. Ties are
/// broken by the smallest community id. Hubs with no neighbours in any
/// community get their own fresh community (`next_cid`, which is then
/// incremented).
pub(crate) fn reattach_hubs(
    hub_nodes: IndexSet<String>,
    all_edges: &[(String, String, f64)],
    raw: &mut IndexMap<i64, Vec<String>>,
    next_cid: &mut i64,
) {
    let mut node_community: IndexMap<String, i64> = IndexMap::new();
    for (&cid, nodes) in raw.iter() {
        for n in nodes {
            node_community.insert(n.clone(), cid);
        }
    }

    let mut sorted_hubs: Vec<String> = hub_nodes.into_iter().collect();
    sorted_hubs.sort_unstable();

    for hub in sorted_hubs {
        let mut votes: IndexMap<i64, usize> = IndexMap::new();
        for (u, v, _) in all_edges {
            let nb = if u == &hub {
                Some(v.as_str())
            } else if v == &hub {
                Some(u.as_str())
            } else {
                None
            };
            if let Some(nb_id) = nb
                && let Some(&cid) = node_community.get(nb_id)
            {
                *votes.entry(cid).or_insert(0) += 1;
            }
        }

        if votes.is_empty() {
            raw.insert(*next_cid, vec![hub.clone()]);
            node_community.insert(hub, *next_cid);
            *next_cid += 1;
        } else {
            let best = votes
                .iter()
                .max_by(|(c1, v1), (c2, v2)| v1.cmp(v2).then_with(|| c2.cmp(c1)))
                .map_or(0, |(&c, _)| c); // votes is non-empty; map_or fallback unreachable
            raw.entry(best).or_default().push(hub.clone());
            node_community.insert(hub, best);
        }
    }
}
