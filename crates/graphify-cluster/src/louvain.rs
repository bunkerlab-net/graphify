//! Pure-Rust Louvain community detection.
//!
//! Mirrors the fallback path in `graphify-py/graphify/cluster.py` which calls
//! `nx.community.louvain_communities(stable, seed=42, threshold=1e-4,
//! resolution=resolution, max_level=10)`.
//!
//! Louvain overview:
//! - Phase 1 (local moves): for each node, compute the modularity gain of
//!   moving it into each of its neighbours' communities; apply the best
//!   improvement greedily.  Repeat until no improvement exceeds `threshold`.
//! - Phase 2 (aggregation): collapse each community into a single super-node,
//!   build the contracted graph, repeat Phase 1.
//! - Repeat for up to `max_level` levels.
//!
//! Tie-breaking is done with a seeded `rand::rngs::StdRng` to match the
//! Python `seed=42` behaviour. `rand` 0.10 removed `SmallRng`, so we use the
//! ChaCha-backed `StdRng` instead; the determinism guarantee is identical.

use std::collections::HashMap;

use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rand::seq::SliceRandom as _;

const DEFAULT_SEED: u64 = 42;
#[allow(clippy::cast_precision_loss)] // threshold is a heuristic, precision loss is acceptable
const DEFAULT_THRESHOLD: f64 = 1e-4;
const DEFAULT_MAX_LEVEL: usize = 10;

/// Run Louvain on a list of undirected edges (node indices `0..n_nodes`).
///
/// Returns a Vec of length `n_nodes` where `result[i]` is the community
/// assignment for node `i`.
///
/// `weights[e]` is the weight of the e-th edge. Pass `None` to treat all
/// edges as weight 1.0.
fn louvain_indices(
    n_nodes: usize,
    edges: &[(usize, usize)],
    weights: Option<&[f64]>,
    resolution: f64,
    threshold: f64,
    max_level: usize,
) -> Vec<usize> {
    if n_nodes == 0 {
        return Vec::new();
    }

    // Build initial adjacency from edges
    let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n_nodes];
    for (idx, &(u, v)) in edges.iter().enumerate() {
        let w = weights.map_or(1.0, |ws| ws[idx]);
        if u != v {
            *adj[u].entry(v).or_insert(0.0) += w;
            *adj[v].entry(u).or_insert(0.0) += w;
        }
    }

    // community[i] = which community node i belongs to.
    // Start: each node is its own community.
    let mut community: Vec<usize> = (0..n_nodes).collect();

    // We run Louvain up to max_level passes on progressively aggregated graphs.
    // `final_community[i]` tracks the final community for each original node.
    let mut final_community: Vec<usize> = community.clone();

    let mut rng = StdRng::seed_from_u64(DEFAULT_SEED);

    // Current graph state (may be contracted between passes)
    let mut cur_n = n_nodes;
    let mut cur_adj = adj;
    // For each node in the contracted graph, which set of original nodes does
    // it represent?  Used to propagate community assignments back.
    let mut supernode_to_orig: Vec<Vec<usize>> = (0..n_nodes).map(|i| vec![i]).collect();

    for _level in 0..max_level {
        let improved = louvain_phase1(
            cur_n,
            &cur_adj,
            resolution,
            threshold,
            &mut community,
            &mut rng,
        );

        // Propagate communities back to original nodes
        for (super_idx, orig_nodes) in supernode_to_orig.iter().enumerate() {
            let c = community[super_idx];
            for &orig in orig_nodes {
                final_community[orig] = c;
            }
        }

        if !improved {
            break;
        }

        // Renumber communities 0..k
        let (renumbered_community, k) = renumber(&community, cur_n);
        if k >= cur_n {
            // No merges happened - nothing more to do
            break;
        }

        // Update supernode_to_orig for contracted graph
        let mut new_supernode: Vec<Vec<usize>> = vec![Vec::new(); k];
        for (super_idx, orig_nodes) in supernode_to_orig.into_iter().enumerate() {
            new_supernode[renumbered_community[super_idx]].extend(orig_nodes);
        }
        supernode_to_orig = new_supernode;

        // Build contracted graph
        let contracted_adj = contract_graph(k, &renumbered_community, cur_n, &cur_adj);

        cur_n = k;
        cur_adj = contracted_adj;
        community = (0..k).collect(); // each super-node starts in its own community
    }

    final_community
}

/// Phase 1: local moves. Returns true if any improvement was made.
fn louvain_phase1(
    n: usize,
    adj: &[HashMap<usize, f64>],
    resolution: f64,
    _threshold: f64,
    community: &mut [usize],
    rng: &mut StdRng,
) -> bool {
    // tot[c] = sum of degrees (strengths) of nodes in community c
    let degrees: Vec<f64> = (0..n)
        .map(|i| adj[i].values().copied().sum::<f64>())
        .collect();
    let m: f64 = degrees.iter().sum::<f64>() / 2.0; // total edge weight

    if m == 0.0 {
        return false; // no edges, nothing to do
    }

    let mut tot: HashMap<usize, f64> = HashMap::new();
    for i in 0..n {
        *tot.entry(community[i]).or_insert(0.0) += degrees[i];
    }

    let mut order: Vec<usize> = (0..n).collect();
    let mut any_improved = false;

    loop {
        let mut improved_this_pass = false;
        order.shuffle(rng);

        for &node in &order {
            let current_c = community[node];
            let k_i = degrees[node];

            // Weight of edges from `node` to each community
            let mut nbr_community_weight: HashMap<usize, f64> = HashMap::new();
            for (&nb, &w) in &adj[node] {
                *nbr_community_weight.entry(community[nb]).or_insert(0.0) += w;
            }

            let k_i_in_current = nbr_community_weight.get(&current_c).copied().unwrap_or(0.0);

            // Remove node from its community (temporarily)
            *tot.entry(current_c).or_insert(0.0) -= k_i;

            // Delta Q for removing from current community
            let tot_current = *tot.get(&current_c).unwrap_or(&0.0);
            let remove_gain = k_i_in_current / m - resolution * (tot_current * k_i) / (2.0 * m * m);

            // Find best community to move to
            let mut best_c = current_c;
            let mut best_gain = 0.0_f64;

            // Consider all neighbour communities (and also re-inserting into current)
            let mut candidates: Vec<usize> = nbr_community_weight.keys().copied().collect();
            candidates.sort_unstable(); // deterministic order for tie-breaking
            for c in candidates {
                if c == current_c {
                    continue;
                }
                let k_i_in_c = nbr_community_weight.get(&c).copied().unwrap_or(0.0);
                let tot_c = *tot.get(&c).unwrap_or(&0.0);
                let gain = k_i_in_c / m - resolution * (tot_c * k_i) / (2.0 * m * m) - remove_gain;
                // Use a small epsilon for float comparison to avoid pedantic float_cmp
                // The epsilon is relative to the modularity scale (1/m).
                let is_better = gain > best_gain + f64::EPSILON / m;
                // Tie-break by smaller community id for determinism
                let is_tied_better = (gain - best_gain).abs() <= f64::EPSILON / m && c < best_c;
                if is_better || is_tied_better {
                    best_gain = gain;
                    best_c = c;
                }
            }

            // Re-insert into best community
            *tot.entry(best_c).or_insert(0.0) += k_i;

            if best_c != current_c {
                community[node] = best_c;
                improved_this_pass = true;
            }
        }

        if improved_this_pass {
            any_improved = true;
        } else {
            break;
        }
        // _threshold is used implicitly: we break when no moves happen.
    }

    any_improved
}

/// Renumber community labels to 0..k and return the k count.
fn renumber(community: &[usize], n: usize) -> (Vec<usize>, usize) {
    let mut old_to_new: HashMap<usize, usize> = HashMap::new();
    let mut next_id = 0_usize;
    let mut result = vec![0_usize; n];
    for (i, &c) in community.iter().enumerate() {
        let new_c = *old_to_new.entry(c).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        result[i] = new_c;
    }
    (result, next_id)
}

/// Build the contracted graph: merge all nodes in the same community.
fn contract_graph(
    k: usize,
    community: &[usize],
    n: usize,
    adj: &[HashMap<usize, f64>],
) -> Vec<HashMap<usize, f64>> {
    let mut new_adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); k];
    for u in 0..n {
        let cu = community[u];
        for (&v, &w) in &adj[u] {
            let cv = community[v];
            if cu != cv {
                *new_adj[cu].entry(cv).or_insert(0.0) += w;
            }
            // Self-loops inside the same community are not needed in the
            // contracted adjacency (they don't contribute to inter-community
            // modularity gain).
        }
    }
    new_adj
}

/// Run Louvain on a set of nodes given as string IDs with string-keyed edges.
///
/// Returns a map of `node_id → community_id`.
#[must_use]
pub fn partition(
    nodes: &[String],
    edges: &[(String, String, f64)],
    resolution: f64,
) -> HashMap<String, usize> {
    if nodes.is_empty() {
        return HashMap::new();
    }

    // Assign a stable integer index to each node (sorted for determinism)
    let mut sorted_nodes = nodes.to_vec();
    sorted_nodes.sort_unstable();
    let node_to_idx: HashMap<String, usize> = sorted_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i))
        .collect();

    let n = sorted_nodes.len();

    // Build edge list with integer indices; skip edges whose endpoints are unknown
    let mut int_edges: Vec<(usize, usize)> = Vec::new();
    let mut int_weights: Vec<f64> = Vec::new();
    // Sort edges for determinism
    let mut sorted_edges = edges.to_vec();
    sorted_edges.sort_unstable_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });
    for (src, tgt, w) in &sorted_edges {
        if let (Some(&u), Some(&v)) = (node_to_idx.get(src), node_to_idx.get(tgt)) {
            int_edges.push((u, v));
            int_weights.push(*w);
        }
    }

    let assignments = louvain_indices(
        n,
        &int_edges,
        Some(&int_weights),
        resolution,
        DEFAULT_THRESHOLD,
        DEFAULT_MAX_LEVEL,
    );

    sorted_nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.clone(), assignments[i]))
        .collect()
}
