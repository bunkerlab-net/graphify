//! Community ID remapping.
//!
//! Ports `remap_communities_to_previous` from
//! `graphify-py/graphify/cluster.py`.

use indexmap::IndexMap;

/// Remap community IDs to maximise overlap with a previous assignment.
///
/// Uses greedy one-to-one matching by intersection size, then assigns fresh
/// IDs to unmatched communities in deterministic order (size desc, lexical
/// tie-break on sorted node list).
///
/// `previous` maps `node_id → old_community_id`.
///
/// Returns a new `IndexMap` sorted by community ID ascending.
#[must_use]
pub fn remap_communities_to_previous(
    communities: &IndexMap<i64, Vec<String>>,
    previous: &IndexMap<String, i64>,
) -> IndexMap<i64, Vec<String>> {
    if communities.is_empty() {
        return IndexMap::new();
    }

    // Build sets for new communities
    let new_sets: IndexMap<i64, indexmap::IndexSet<&str>> = communities
        .iter()
        .map(|(&cid, nodes)| {
            let set: indexmap::IndexSet<&str> = nodes.iter().map(String::as_str).collect();
            (cid, set)
        })
        .collect();

    // Build sets for old communities from the previous map
    let mut old_sets: IndexMap<i64, indexmap::IndexSet<&str>> = IndexMap::new();
    for (node, &old_cid) in previous {
        old_sets.entry(old_cid).or_default().insert(node.as_str());
    }

    // Compute pairwise overlaps
    let mut overlaps: Vec<(usize, i64, i64)> = Vec::new(); // (overlap, old_cid, new_cid)
    for (&old_cid, old_nodes) in &old_sets {
        for (&new_cid, new_nodes) in &new_sets {
            let overlap = old_nodes.iter().filter(|n| new_nodes.contains(*n)).count();
            if overlap > 0 {
                overlaps.push((overlap, old_cid, new_cid));
            }
        }
    }
    // Sort: descending overlap, then ascending old_cid, then ascending new_cid
    overlaps.sort_unstable_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    // Greedy one-to-one matching
    let mut new_to_final: IndexMap<i64, i64> = IndexMap::new();
    let mut used_old_ids: indexmap::IndexSet<i64> = indexmap::IndexSet::new();
    let mut matched_new_ids: indexmap::IndexSet<i64> = indexmap::IndexSet::new();

    for (_, old_cid, new_cid) in &overlaps {
        if used_old_ids.contains(old_cid) || matched_new_ids.contains(new_cid) {
            continue;
        }
        new_to_final.insert(*new_cid, *old_cid);
        used_old_ids.insert(*old_cid);
        matched_new_ids.insert(*new_cid);
    }

    // Unmatched communities get fresh IDs, ordered by size desc then lexical
    let mut unmatched: Vec<i64> = communities
        .keys()
        .copied()
        .filter(|cid| !matched_new_ids.contains(cid))
        .collect();
    // Pre-sort node lists once so the comparator is allocation-free.
    let sorted_nodes_by_cid: IndexMap<i64, Vec<String>> = unmatched
        .iter()
        .map(|cid| {
            let mut sorted = communities[cid].clone();
            sorted.sort_unstable();
            (*cid, sorted)
        })
        .collect();
    unmatched.sort_unstable_by(|a, b| {
        let size_a = communities[a].len();
        let size_b = communities[b].len();
        size_b
            .cmp(&size_a)
            .then_with(|| sorted_nodes_by_cid[a].cmp(&sorted_nodes_by_cid[b]))
    });

    let mut next_id: i64 = 0;
    for new_cid in unmatched {
        while used_old_ids.contains(&next_id) {
            next_id += 1;
        }
        new_to_final.insert(new_cid, next_id);
        used_old_ids.insert(next_id);
        next_id += 1;
    }

    // Build result sorted by final community ID ascending
    let mut remapped: IndexMap<i64, Vec<String>> = IndexMap::new();
    for (&new_cid, nodes) in communities {
        if let Some(&final_cid) = new_to_final.get(&new_cid) {
            let mut sorted_nodes = nodes.clone();
            sorted_nodes.sort_unstable();
            remapped.insert(final_cid, sorted_nodes);
        }
    }
    remapped.sort_unstable_keys();
    remapped
}
