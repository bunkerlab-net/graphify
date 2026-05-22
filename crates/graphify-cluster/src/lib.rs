//! Community detection on `graphify_build::Graph` values.
//!
//! Ports `graphify-py/graphify/cluster.py`.
//!
//! ## Algorithm
//!
//! The Python reference attempts Leiden (graspologic) and falls back to
//! `NetworkX`'s Louvain.  This crate ships a pure-Rust Louvain implementation
//! (see `louvain.rs`) seeded with `rand::rngs::SmallRng` at seed 42 — the
//! same seed the Python fallback uses.  Leiden is intentionally **not**
//! implemented; no suitable Rust crate exists in the workspace and the
//! structural-correctness tests do not require identical community IDs.
//!
//! See `.claude/local/notes/module_cluster.md` for a full rationale.

mod cohesion;
mod louvain;
mod remap;

pub use cohesion::{cohesion_score, score_all};
pub use remap::remap_communities_to_previous;

use graphify_build::{Graph, GraphKind};
use indexmap::IndexMap;

// ── Tuning constants (mirror Python) ────────────────────────────────────────

/// Communities larger than this fraction of graph nodes get split.
const MAX_COMMUNITY_FRACTION: f64 = 0.25;
/// Only split a community if it has at least this many nodes.
const MIN_SPLIT_SIZE: usize = 10;
/// Re-split communities with cohesion below this threshold.
const COHESION_SPLIT_THRESHOLD: f64 = 0.05;
/// Only apply cohesion split to communities with at least this many nodes.
const COHESION_SPLIT_MIN_SIZE: usize = 50;

// ── Internal graph helpers ───────────────────────────────────────────────────

/// Build a node list and undirected edge list from a `Graph`.
///
/// If the graph is directed, each directed edge is turned into an undirected
/// one (duplicate pairs are de-duplicated by keeping the maximum weight).
fn to_undirected_edge_list(graph: &Graph) -> (Vec<String>, Vec<(String, String, f64)>) {
    let nodes: Vec<String> = graph.nodes().map(|(id, _)| id.clone()).collect();

    let mut edge_map: IndexMap<(String, String), f64> = IndexMap::new();
    for edge in graph.edges() {
        let (u, v) = if edge.source <= edge.target {
            (edge.source.clone(), edge.target.clone())
        } else {
            (edge.target.clone(), edge.source.clone())
        };
        let w = edge
            .attrs
            .get("weight")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0);
        let entry = edge_map.entry((u, v)).or_insert(0.0);
        if w > *entry {
            *entry = w;
        }
    }

    let edges: Vec<(String, String, f64)> =
        edge_map.into_iter().map(|((u, v), w)| (u, v, w)).collect();

    (nodes, edges)
}

/// Build a subgraph node/edge list for the given subset of nodes.
fn subgraph_edge_list(
    graph: &Graph,
    subset: &[String],
) -> (Vec<String>, Vec<(String, String, f64)>) {
    let node_set: indexmap::IndexSet<&str> = subset.iter().map(String::as_str).collect();

    let nodes = subset.to_vec();

    let mut edge_map: IndexMap<(String, String), f64> = IndexMap::new();
    for edge in graph.edges() {
        if !node_set.contains(edge.source.as_str()) || !node_set.contains(edge.target.as_str()) {
            continue;
        }
        let (u, v) = if edge.source <= edge.target {
            (edge.source.clone(), edge.target.clone())
        } else {
            (edge.target.clone(), edge.source.clone())
        };
        let w = edge
            .attrs
            .get("weight")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0);
        let entry = edge_map.entry((u, v)).or_insert(0.0);
        if w > *entry {
            *entry = w;
        }
    }

    let edges: Vec<(String, String, f64)> =
        edge_map.into_iter().map(|((u, v), w)| (u, v, w)).collect();

    (nodes, edges)
}

/// Run Louvain on the given node/edge list and return `{node_id → community_id}`.
fn run_partition(
    nodes: &[String],
    edges: &[(String, String, f64)],
    resolution: f64,
) -> IndexMap<String, i64> {
    let raw = louvain::partition(nodes, edges, resolution);
    // community IDs from Louvain are small indices; casting usize→i64 is safe
    // for any realistic graph (community index bounded by node count).
    #[allow(clippy::cast_possible_wrap)] // community IDs are small indices bounded by node count
    raw.into_iter()
        .map(|(node, cid)| (node, cid as i64))
        .collect()
}

// ── Community splitting ──────────────────────────────────────────────────────

/// Run a second Louvain pass on a community subgraph to split it further.
///
/// Returns a list of node-lists; each inner list is one sub-community.
/// If the subgraph has no edges the nodes are returned as individual singletons.
/// If the second pass yields only one community the original list is returned
/// unchanged (no artificial split).
fn split_community(graph: &Graph, nodes: &[String]) -> Vec<Vec<String>> {
    let (sub_nodes, sub_edges) = subgraph_edge_list(graph, nodes);

    if sub_edges.is_empty() {
        // No edges — split into individual nodes
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

// ── Hub utilities ─────────────────────────────────────────────────────────────

/// Compute the set of hub nodes to exclude, given percentile threshold.
fn compute_hub_nodes(degree_map: &IndexMap<String, usize>, pct: f64) -> indexmap::IndexSet<String> {
    let mut degrees: Vec<usize> = degree_map.values().copied().collect();
    degrees.sort_unstable();
    if degrees.is_empty() {
        return indexmap::IndexSet::new();
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
fn reattach_hubs(
    hub_nodes: indexmap::IndexSet<String>,
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
            // best = max votes, tie-break by smallest community id
            let best = votes
                .iter()
                .max_by(|(c1, v1), (c2, v2)| v1.cmp(v2).then_with(|| c2.cmp(c1)))
                .map_or(0, |(&c, _)| c); // votes is non-empty; map_or fallback unreachable
            raw.entry(best).or_default().push(hub.clone());
            node_community.insert(hub, best);
        }
    }
}

/// Apply oversized-community splitting and low-cohesion re-splitting.
fn apply_splits(communities: Vec<Vec<String>>, graph: &Graph, max_size: usize) -> Vec<Vec<String>> {
    // Split oversized communities
    let mut after_size: Vec<Vec<String>> = Vec::new();
    for nodes in communities {
        if nodes.len() > max_size {
            after_size.extend(split_community(graph, &nodes));
        } else {
            after_size.push(nodes);
        }
    }

    // Second pass: re-split low-cohesion communities
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

// ── Public API ───────────────────────────────────────────────────────────────

/// Run Louvain community detection on `graph`.
///
/// Returns `{community_id: [node_ids]}` sorted by community size descending
/// (community 0 is largest).  Nodes within each community are sorted
/// alphabetically.
///
/// - Empty graph → empty map.
/// - `DiGraph` is converted to undirected internally.
/// - Graph with no edges → each node is its own community.
/// - Oversized communities (>25 % of graph nodes, min 10) are split via a
///   second Louvain pass.
/// - Low-cohesion communities (cohesion < 0.05, size ≥ 50) are re-split.
/// - If `exclude_hubs_percentile` is set (0–100), nodes whose degree exceeds
///   that percentile are excluded from the initial partition and re-attached
///   via majority-neighbour vote.
///
/// ## Note on Leiden
///
/// The Python reference uses Leiden (graspologic) and falls back to Louvain.
/// This crate implements only Louvain; Leiden is skipped.  See
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

    // Convert directed graph to undirected (Louvain requires undirected input).
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

    // Degree map for hub exclusion and isolate detection
    let mut degree_map: IndexMap<String, usize> = IndexMap::new();
    for n in &all_nodes {
        degree_map.insert(n.clone(), 0);
    }
    for (u, v, _) in &all_edges {
        *degree_map.entry(u.clone()).or_insert(0) += 1;
        *degree_map.entry(v.clone()).or_insert(0) += 1;
    }

    let hub_nodes: indexmap::IndexSet<String> = exclude_hubs_percentile
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

    let connected_set: indexmap::IndexSet<&str> =
        connected_nodes.iter().map(String::as_str).collect();
    let connected_edges: Vec<(String, String, f64)> = all_edges
        .iter()
        .filter(|(u, v, _)| {
            connected_set.contains(u.as_str()) && connected_set.contains(v.as_str())
        })
        .cloned()
        .collect();

    // Initial partition
    let mut raw: IndexMap<i64, Vec<String>> = IndexMap::new();
    if !connected_nodes.is_empty() {
        let partition = run_partition(&connected_nodes, &connected_edges, resolution);
        for (node, cid) in partition {
            raw.entry(cid).or_default().push(node);
        }
    }

    // Assign each isolate its own community
    let mut next_cid: i64 = raw.keys().copied().max().map_or(0, |m| m + 1);
    for node in isolates {
        raw.insert(next_cid, vec![node]);
        next_cid += 1;
    }

    // Re-attach hub nodes
    if !hub_nodes.is_empty() {
        reattach_hubs(hub_nodes, &all_edges, &mut raw, &mut next_cid);
    }

    // Compute max community size threshold
    // node_count bounded; precision loss and sign loss are acceptable here
    #[allow(clippy::cast_precision_loss)] // node count bounded; precision loss acceptable
    #[allow(clippy::cast_possible_truncation)] // intentional floor for max size
    #[allow(clippy::cast_sign_loss)] // MAX_COMMUNITY_FRACTION is positive constant
    let max_size = MIN_SPLIT_SIZE.max((g.node_count() as f64 * MAX_COMMUNITY_FRACTION) as usize);

    let mut final_communities = apply_splits(raw.into_values().collect(), g, max_size);

    // Final ordering: size desc, nodes within community sorted alphabetically
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
