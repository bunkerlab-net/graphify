//! Cohesion score computation.
//!
//! Ports `cohesion_score` and `score_all` from
//! `graphify-py/graphify/cluster.py`.

use graphify_build::Graph;
use indexmap::IndexMap;

/// Ratio of actual intra-community edges to the maximum possible.
///
/// Returns `1.0` for communities of size ≤ 1 (a single node is perfectly
/// cohesive by definition).  Returns `0.0` for communities with no internal
/// edges.
///
/// Mirrors Python: `actual / (n*(n-1)/2)`.
#[must_use]
pub fn cohesion_score(graph: &Graph, community_nodes: &[String]) -> f64 {
    let n = community_nodes.len();
    if n <= 1 {
        return 1.0;
    }

    // Build a set for O(1) membership tests
    let node_set: indexmap::IndexSet<&str> = community_nodes.iter().map(String::as_str).collect();

    // Count edges whose both endpoints are in the community.
    // For undirected graphs each edge is stored once so we count it once.
    // For directed graphs the Python implementation uses `G.subgraph(nodes)`
    // which is *undirected* after the DiGraph→Graph conversion that cluster()
    // performs, so we deduplicate (u,v) / (v,u) pairs.
    let directed = graph.kind.is_directed();
    let mut actual: usize = 0;

    if directed {
        // Count only unique unordered pairs to mirror undirected cohesion
        use std::collections::HashSet;
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        for edge in graph.edges() {
            let src = edge.source.as_str();
            let tgt = edge.target.as_str();
            if node_set.contains(src) && node_set.contains(tgt) {
                let key = if src <= tgt { (src, tgt) } else { (tgt, src) };
                if seen.insert(key) {
                    actual += 1;
                }
            }
        }
    } else {
        for edge in graph.edges() {
            if node_set.contains(edge.source.as_str()) && node_set.contains(edge.target.as_str()) {
                actual += 1;
            }
        }
    }

    // n and actual are at most graph size; n*(n-1)/2 fits in u64 well within
    // the range representable exactly by f64 (which has 53-bit mantissa) for
    // all practical graph sizes (< 2^26 nodes). Allowing cast_precision_loss
    // here is intentional and acceptable.
    #[allow(clippy::cast_precision_loss)] // graph sizes are bounded; precision loss is negligible
    let possible = (n * (n - 1)) as f64 / 2.0;
    #[allow(clippy::cast_precision_loss)] // edge counts are bounded; precision loss is negligible
    let actual_f = actual as f64;
    if possible > 0.0 {
        actual_f / possible
    } else {
        0.0
    }
}

/// Compute cohesion scores for every community in the map.
///
/// Returns an `IndexMap<community_id, score>` with the same key set as
/// `communities`.
///
/// Runs in `O(N + E + C)` rather than the naive `O(C × E)` of calling
/// [`cohesion_score`] per community. For a graph with 25k nodes / 36k edges
/// and 785 communities, the naive shape costs ~28M edge iterations; the
/// single-pass version is one walk over the edge list plus one over the
/// community map.
#[must_use]
pub fn score_all(graph: &Graph, communities: &IndexMap<i64, Vec<String>>) -> IndexMap<i64, f64> {
    if communities.is_empty() {
        return IndexMap::new();
    }

    // node_id → community_id (one lookup per edge endpoint instead of
    // O(C × N) set rebuilds).
    let mut node_to_cid: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for (&cid, nodes) in communities {
        for n in nodes {
            node_to_cid.insert(n.as_str(), cid);
        }
    }

    // For directed graphs we deduplicate (u, v) / (v, u) pairs per-community
    // to match the Python reference, which converts DiGraph→Graph before
    // subgraphing.
    let directed = graph.kind.is_directed();
    let mut actual: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    // Only directed graphs need the duplicate-pair guard; undirected graphs
    // store each edge once.
    let mut seen_directed: Option<std::collections::HashSet<(i64, &str, &str)>> =
        directed.then(std::collections::HashSet::new);

    for edge in graph.edges() {
        let src = edge.source.as_str();
        let tgt = edge.target.as_str();
        let (Some(&cu), Some(&cv)) = (node_to_cid.get(src), node_to_cid.get(tgt)) else {
            continue;
        };
        if cu != cv {
            continue;
        }
        if let Some(ref mut seen) = seen_directed {
            // Order-insensitive key for undirected counting under directed storage.
            let (a, b) = if src <= tgt { (src, tgt) } else { (tgt, src) };
            if !seen.insert((cu, a, b)) {
                continue;
            }
        }
        *actual.entry(cu).or_insert(0) += 1;
    }

    communities
        .iter()
        .map(|(&cid, nodes)| {
            let n = nodes.len();
            if n <= 1 {
                return (cid, 1.0);
            }
            // n*(n-1)/2 fits in u64 well within f64 mantissa for any
            // realistic graph size.
            #[allow(clippy::cast_precision_loss)]
            let possible = (n * (n - 1)) as f64 / 2.0;
            #[allow(clippy::cast_precision_loss)]
            let actual_f = actual.get(&cid).copied().unwrap_or(0) as f64;
            let score = if possible > 0.0 {
                actual_f / possible
            } else {
                0.0
            };
            (cid, score)
        })
        .collect()
}
