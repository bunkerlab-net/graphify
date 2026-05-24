//! Graph centrality algorithms.
//!
//! Extracted from `lib.rs` to isolate `all_degrees`, `neighbors`,
//! `betweenness_centrality`, `edge_betweenness_centrality`,
//! `build_neighbor_indices`, and `cohesion_score`.

use graphify_build::Graph;
use indexmap::IndexMap;

/// Compute degree (count of incident edges) for every node.
///
/// Mirrors `dict(G.degree())` for undirected graphs.
pub(crate) fn all_degrees(graph: &Graph) -> IndexMap<String, usize> {
    let mut deg: IndexMap<String, usize> = IndexMap::new();
    // Initialise every node at 0
    for (id, _) in graph.nodes() {
        deg.insert(id.clone(), 0);
    }
    for edge in graph.edges() {
        *deg.entry(edge.source.clone()).or_insert(0) += 1;
        if edge.source != edge.target {
            *deg.entry(edge.target.clone()).or_insert(0) += 1;
        }
    }
    deg
}

/// Return the neighbours of `node_id`.
pub(crate) fn neighbors<'a>(graph: &'a Graph, node_id: &str) -> Vec<&'a str> {
    let directed = graph.kind.is_directed();
    let mut out: Vec<&str> = Vec::new();
    for edge in graph.edges() {
        if edge.source == node_id {
            out.push(&edge.target);
        } else if !directed && edge.target == node_id {
            out.push(&edge.source);
        }
    }
    out
}

/// Compute approximate or exact betweenness centrality (Brandes' algorithm).
///
/// When `k` is `Some(k)`, uses `k` random pivot nodes (sampled in insertion
/// order, no actual randomness needed for determinism, we take the first k).
/// Mirrors Python `nx.betweenness_centrality(G, k=k, seed=42)`.
///
/// Returns `node_id → centrality` (normalised by `1 / ((n-1)(n-2)/2)` for
/// undirected graphs).
#[allow(clippy::cast_precision_loss)] // graph node counts fit well within f64 mantissa in practice
pub(crate) fn betweenness_centrality(graph: &Graph, k: Option<usize>) -> IndexMap<String, f64> {
    let nodes: Vec<&String> = graph.node_map.keys().collect();
    let n = nodes.len();
    let mut betweenness: IndexMap<String, f64> =
        nodes.iter().map(|&id| (id.clone(), 0.0_f64)).collect();

    if n < 2 {
        return betweenness;
    }

    // Build adjacency for quick lookup
    let directed = graph.kind.is_directed();

    // Index nodes
    let node_idx: IndexMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, &id)| (id.as_str(), i))
        .collect();

    let pivot_count = k.unwrap_or(n).min(n);

    // For each source, run BFS and accumulate pair-dependency
    for s_idx in 0..pivot_count {
        let s = nodes[s_idx].as_str();

        let mut stack: Vec<usize> = Vec::new();
        let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut sigma: Vec<f64> = vec![0.0; n];
        let mut dist: Vec<i64> = vec![-1; n];

        let s_i = node_idx[s];
        sigma[s_i] = 1.0;
        dist[s_i] = 0;

        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        queue.push_back(s_i);

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let v_id = nodes[v].as_str();
            let nbrs = build_neighbor_indices(graph, v_id, &node_idx, directed);
            for w in nbrs {
                if dist[w] < 0 {
                    queue.push_back(w);
                    dist[w] = dist[v] + 1;
                }
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }

        let mut delta: Vec<f64> = vec![0.0; n];
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                if sigma[w] > 0.0 {
                    delta[v] += (sigma[v] / sigma[w]) * (1.0 + delta[w]);
                }
            }
            if w != s_i {
                let id = nodes[w].clone();
                *betweenness.entry(id).or_insert(0.0) += delta[w];
            }
        }
    }

    // Normalise
    let scale = if n > 2 {
        let factor = if directed {
            1.0 / ((n - 1) as f64 * (n - 2) as f64)
        } else {
            2.0 / ((n - 1) as f64 * (n - 2) as f64)
        };
        if k.is_some() {
            // Rescale for sampling (multiply by n/k)
            factor * (n as f64 / pivot_count as f64)
        } else {
            factor
        }
    } else {
        1.0
    };

    for v in betweenness.values_mut() {
        *v *= scale;
    }

    betweenness
}

/// Build list of neighbour indices for betweenness BFS.
pub(crate) fn build_neighbor_indices(
    graph: &Graph,
    node_id: &str,
    node_idx: &IndexMap<&str, usize>,
    directed: bool,
) -> Vec<usize> {
    let mut out = Vec::new();
    for edge in graph.edges() {
        if edge.source == node_id {
            if let Some(&i) = node_idx.get(edge.target.as_str()) {
                out.push(i);
            }
        } else if !directed
            && edge.target == node_id
            && let Some(&i) = node_idx.get(edge.source.as_str())
        {
            out.push(i);
        }
    }
    out
}

/// Compute edge betweenness centrality.
///
/// Mirrors Python `nx.edge_betweenness_centrality(G)`.
#[allow(clippy::cast_precision_loss)] // graph node counts fit well within f64 mantissa in practice
pub(crate) fn edge_betweenness_centrality(graph: &Graph) -> Vec<((String, String), f64)> {
    let nodes: Vec<&String> = graph.node_map.keys().collect();
    let n = nodes.len();

    if n < 2 {
        return Vec::new();
    }

    let directed = graph.kind.is_directed();
    let node_idx: IndexMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, &id)| (id.as_str(), i))
        .collect();

    // Map edge pair → betweenness accumulator
    let mut edge_bet: IndexMap<(usize, usize), f64> = IndexMap::new();
    // Initialise all edges
    for edge in graph.edges() {
        if let (Some(&u), Some(&v)) = (
            node_idx.get(edge.source.as_str()),
            node_idx.get(edge.target.as_str()),
        ) {
            let key = if directed || u < v { (u, v) } else { (v, u) };
            edge_bet.entry(key).or_insert(0.0);
        }
    }

    for s_idx in 0..n {
        let s = nodes[s_idx].as_str();
        let s_i = node_idx[s];

        let mut stack: Vec<usize> = Vec::new();
        let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut sigma: Vec<f64> = vec![0.0; n];
        let mut dist: Vec<i64> = vec![-1; n];

        sigma[s_i] = 1.0;
        dist[s_i] = 0;

        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        queue.push_back(s_i);

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let v_id = nodes[v].as_str();
            let nbrs = build_neighbor_indices(graph, v_id, &node_idx, directed);
            for w in nbrs {
                if dist[w] < 0 {
                    queue.push_back(w);
                    dist[w] = dist[v] + 1;
                }
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }

        let mut delta: Vec<f64> = vec![0.0; n];
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                if sigma[w] > 0.0 {
                    let contribution = (sigma[v] / sigma[w]) * (1.0 + delta[w]);
                    delta[v] += contribution;
                    if w != s_i {
                        let key = if directed || v < w { (v, w) } else { (w, v) };
                        *edge_bet.entry(key).or_insert(0.0) += contribution;
                    }
                }
            }
        }
    }

    // Normalise and convert back to string keys
    let scale = if n > 1 {
        if directed {
            1.0 / ((n - 1) as f64 * n as f64)
        } else {
            2.0 / ((n - 1) as f64 * n as f64)
        }
    } else {
        1.0
    };

    edge_bet
        .into_iter()
        .map(|((u, v), b)| ((nodes[u].clone(), nodes[v].clone()), b * scale))
        .collect()
}
