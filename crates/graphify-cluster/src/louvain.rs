//! Pure-Rust Louvain community detection used as the sole partitioning backend.
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
//!
//! Performance: the hot path uses `Vec<Vec<(usize, f64)>>` adjacency and
//! pre-allocated `Vec<f64>` scratch buffers (community totals + per-node
//! neighbour-weight accumulators) so per-node moves are alloc-free. On a
//! 16k-node graph this is roughly two orders of magnitude faster than the
//! prior `HashMap`-based implementation. Phase-1 moves remain sequential —
//! moving node A changes the modularity score for node B — but the
//! O(N+E) graph contraction between levels is parallelised with rayon when
//! the contracted graph is large enough to amortise the fan-out cost.

use std::collections::HashMap;

use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rand::seq::SliceRandom as _;
use rayon::prelude::*;

/// Below this number of supernodes, `contract_graph` runs sequentially —
/// rayon's thread-pool fan-out costs more than the sequential work for tiny
/// graphs. Conservative threshold; matches the bands used in other crates
/// (see `.claude/local/notes/perf_rayon.md`).
const CONTRACT_PARALLEL_THRESHOLD: usize = 4096;

/// Hard cap on the per-level Phase 1 inner pass count.
///
/// Phase 1's natural convergence criterion ("no node moved in a full pass")
/// can in principle oscillate forever when groups of nodes flip-flop between
/// neighbouring communities — A → X, B → Y, then C reattaches A back to A's
/// original community, and so on. The pathology is well-known; scikit-network,
/// igraph, and graspologic all cap the inner loop. `NetworkX` does not, which
/// is the outlier behaviour we deliberately diverge from.
///
/// 100 passes is enough to converge on every well-behaved input we have
/// (production graphs converge in under 10 passes); the cap exists purely
/// so a pathological input can't hang the CLI.
const MAX_INNER_PASSES: usize = 100;

/// RNG seed matching the Python reference `seed=42`.
const DEFAULT_SEED: u64 = 42;
/// Minimum modularity gain required to accept a node move.
#[allow(clippy::cast_precision_loss)] // threshold is a heuristic, precision loss is acceptable
const DEFAULT_THRESHOLD: f64 = 1e-4;
/// Maximum number of Louvain aggregation levels.
const DEFAULT_MAX_LEVEL: usize = 10;

/// Run the full multi-level Louvain algorithm on a list of undirected edges (node indices `0..n_nodes`).
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

    // Build initial adjacency as `Vec<Vec<(usize, f64)>>`. We never look up by
    // neighbour index after building — iteration order is the only thing that
    // matters in the hot loop — so the cache-friendly contiguous layout beats
    // the prior `Vec<HashMap<_, _>>`.
    // Use a transient `HashMap<usize, f64>` per source to dedup edge weights
    // (the same (u, v) pair may appear multiple times from the caller).
    let mut adj_map: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n_nodes];
    for (idx, &(u, v)) in edges.iter().enumerate() {
        let w = weights.map_or(1.0, |ws| ws[idx]);
        if u != v {
            *adj_map[u].entry(v).or_insert(0.0) += w;
            *adj_map[v].entry(u).or_insert(0.0) += w;
        }
    }
    let mut cur_adj: Vec<Vec<(usize, f64)>> = adj_map
        .into_iter()
        .map(|m| m.into_iter().collect())
        .collect();

    // community[i] = which community node i belongs to.
    // Start: each node is its own community.
    let mut community: Vec<usize> = (0..n_nodes).collect();

    // We run Louvain up to max_level passes on progressively aggregated graphs.
    // `final_community[i]` tracks the final community for each original node.
    let mut final_community: Vec<usize> = community.clone();

    let mut rng = StdRng::seed_from_u64(DEFAULT_SEED);

    // Current graph state (may be contracted between passes)
    let mut cur_n = n_nodes;
    // For each node in the contracted graph, which set of original nodes does
    // it represent?  Used to propagate community assignments back.
    let mut supernode_to_orig: Vec<Vec<usize>> = (0..n_nodes).map(|i| vec![i]).collect();

    let progress = std::env::var("GRAPHIFY_CLUSTER_PROGRESS").is_ok_and(|v| !v.is_empty());

    for level in 0..max_level {
        let level_start = std::time::Instant::now();
        let improved = louvain_phase1(
            cur_n,
            &cur_adj,
            resolution,
            threshold,
            &mut community,
            &mut rng,
        );
        if progress {
            eprintln!(
                "      louvain level {level}: {cur_n} supernodes, phase1 {:.2}s",
                level_start.elapsed().as_secs_f64()
            );
        }

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
        cur_adj = contract_graph(k, &renumbered_community, cur_n, &cur_adj);

        cur_n = k;
        community = (0..k).collect(); // each super-node starts in its own community
    }

    final_community
}

/// Execute Phase 1 (local greedy moves) on the current graph state.
///
/// Iterates over nodes in shuffled order, moving each to the neighbour
/// community that yields the greatest modularity gain. Repeats until no
/// node move improves modularity by more than `threshold`. Returns `true`
/// if at least one move was made during the pass.
///
/// The hot loop pre-allocates two `Vec<f64>` scratch buffers of size `n` —
/// one for community totals (`tot`) and one for per-node neighbour-community
/// weight accumulators (`nbr_scratch`) — plus a small `touched` list that
/// records which entries of `nbr_scratch` were dirtied this iteration so
/// they can be cleared in O(touched) rather than O(n).
fn louvain_phase1(
    n: usize,
    adj: &[Vec<(usize, f64)>],
    resolution: f64,
    threshold: f64,
    community: &mut [usize],
    rng: &mut StdRng,
) -> bool {
    // tot[c] = sum of degrees (strengths) of nodes in community c
    let degrees: Vec<f64> = (0..n)
        .map(|i| adj[i].iter().map(|&(_, w)| w).sum())
        .collect();
    let m: f64 = degrees.iter().sum::<f64>() / 2.0; // total edge weight

    if m == 0.0 {
        return false; // no edges, nothing to do
    }

    // Community IDs live in 0..n at every point during this phase (initial
    // assignment is `community[i] = i`, and moves only re-target existing
    // IDs). A plain Vec<f64> indexed by community id is therefore correct.
    let mut tot: Vec<f64> = vec![0.0; n];
    for i in 0..n {
        tot[community[i]] += degrees[i];
    }

    // Scratch buffer used to accumulate edge weights from the current node
    // to each neighbour community. Reused across nodes — entries cleared via
    // the `touched` dirty list.
    let mut nbr_scratch: Vec<f64> = vec![0.0; n];
    let mut touched: Vec<usize> = Vec::with_capacity(32);

    let mut order: Vec<usize> = (0..n).collect();
    let mut any_improved = false;

    let tol = threshold.max(f64::EPSILON / m);
    let two_m_sq = 2.0 * m * m;
    let progress = std::env::var("GRAPHIFY_CLUSTER_PROGRESS").is_ok_and(|v| !v.is_empty());

    for pass in 0..MAX_INNER_PASSES {
        let mut improved_this_pass = false;
        let mut moves_this_pass: usize = 0;
        order.shuffle(rng);

        for &node in &order {
            let current_c = community[node];
            let k_i = degrees[node];

            // Weight of edges from `node` to each community — accumulate into
            // the shared scratch buffer and track touched indices.
            for &(nb, w) in &adj[node] {
                let c = community[nb];
                if nbr_scratch[c] == 0.0 {
                    touched.push(c);
                }
                nbr_scratch[c] += w;
            }

            let k_i_in_current = nbr_scratch[current_c]; // 0 if not touched

            // Remove node from its community (temporarily)
            tot[current_c] -= k_i;

            // Delta Q for removing from current community
            let remove_gain = k_i_in_current / m - resolution * (tot[current_c] * k_i) / two_m_sq;

            // Find best community to move to. We iterate the unsorted `touched`
            // list — the tie-break `c < best_c` already gives "smallest
            // community id wins on equal gain" regardless of visit order, so
            // the explicit `candidates.sort_unstable()` from the old code was
            // redundant.
            let mut best_c = current_c;
            let mut best_gain = 0.0_f64;
            for &c in &touched {
                if c == current_c {
                    continue;
                }
                let k_i_in_c = nbr_scratch[c];
                let gain = k_i_in_c / m - resolution * (tot[c] * k_i) / two_m_sq - remove_gain;
                // Require strictly better gain than the running best by at least
                // `threshold` so NetworkX-style convergence is honoured rather
                // than oscillating on negligible improvements. The constant
                // floor (EPSILON / m) prevents float-cmp flake on perfectly
                // tied gains.
                let is_better = gain > best_gain + tol;
                // Tie-break by smaller community id for determinism.
                let is_tied_better = (gain - best_gain).abs() <= tol && c < best_c;
                if is_better || is_tied_better {
                    best_gain = gain;
                    best_c = c;
                }
            }

            // Re-insert into best community
            tot[best_c] += k_i;

            if best_c != current_c {
                community[node] = best_c;
                improved_this_pass = true;
                moves_this_pass += 1;
            }

            // Clear scratch entries we dirtied so the next node starts clean.
            for &c in &touched {
                nbr_scratch[c] = 0.0;
            }
            touched.clear();
        }

        if progress && (pass < 3 || pass % 10 == 0) {
            eprintln!("        phase1 pass {pass}: {moves_this_pass} moves over {n} nodes");
        }

        if improved_this_pass {
            any_improved = true;
        } else {
            return any_improved;
        }
    }

    // Hit the iteration cap. Emit a warning so the user knows convergence
    // was bounded artificially and the partition for this level is best-
    // effort rather than locally optimal.
    eprintln!(
        "[graphify] cluster: phase1 hit {MAX_INNER_PASSES}-pass cap on {n} nodes; \
         accepting best-effort partition (some communities may be suboptimal)"
    );
    any_improved
}

/// Renumber community labels to the dense range `0..k` and return `k`.
///
/// The input labels may be sparse (e.g. `[0, 5, 5, 12]`); the output is
/// a contiguous re-labelling in first-seen order. Used before graph
/// contraction so super-node indices are tightly packed.
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

/// Build the contracted (super-node) graph for the next Louvain level.
///
/// Each community in `community` becomes a single super-node. Edge weights
/// between super-nodes accumulate the weights of all cross-community edges.
/// Self-loops (intra-community edges) are omitted from the contracted
/// adjacency because they do not contribute to inter-community modularity.
///
/// The sequential path builds each super-node's adjacency via a `HashMap`
/// dedup, then flattens to `Vec<(usize, f64)>`. For large graphs the
/// per-super-node row build is independent, so we fan out via rayon when
/// `k >= CONTRACT_PARALLEL_THRESHOLD`.
fn contract_graph(
    k: usize,
    community: &[usize],
    n: usize,
    adj: &[Vec<(usize, f64)>],
) -> Vec<Vec<(usize, f64)>> {
    // Bucket nodes by their target super-node so each output row can be built
    // independently — this is the parallel decomposition.
    let mut nodes_by_super: Vec<Vec<usize>> = vec![Vec::new(); k];
    for (u, &cu) in community.iter().enumerate().take(n) {
        nodes_by_super[cu].push(u);
    }

    let build_row = |members: &[usize]| -> Vec<(usize, f64)> {
        let mut row: HashMap<usize, f64> = HashMap::new();
        for &u in members {
            let cu = community[u];
            for &(v, w) in &adj[u] {
                let cv = community[v];
                if cu != cv {
                    *row.entry(cv).or_insert(0.0) += w;
                }
            }
        }
        row.into_iter().collect()
    };

    if k >= CONTRACT_PARALLEL_THRESHOLD {
        nodes_by_super
            .par_iter()
            .map(|members| build_row(members))
            .collect()
    } else {
        nodes_by_super
            .iter()
            .map(|members| build_row(members))
            .collect()
    }
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
