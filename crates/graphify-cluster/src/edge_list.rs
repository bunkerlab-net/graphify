//! Convert `graphify_build::Graph` values into the `(nodes, edges)` shape
//! that the pure-Rust Louvain implementation accepts.

use indexmap::IndexMap;

use graphify_build::Graph;

/// Build a node list and undirected edge list from a `Graph`.
///
/// If the graph is directed, each directed edge is turned into an
/// undirected one (duplicate `(u, v)` pairs are de-duplicated by keeping
/// the maximum weight).
pub(crate) fn to_undirected_edge_list(graph: &Graph) -> (Vec<String>, Vec<(String, String, f64)>) {
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

/// Build the node + edge list for a node-induced subgraph of `graph`.
///
/// Edges are kept only when both endpoints are in `subset`. Weights are
/// extracted from the `"weight"` attribute (default 1.0), and duplicate
/// undirected pairs are de-duplicated by keeping the maximum weight.
pub(crate) fn subgraph_edge_list(
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

/// Run the configured community-detection backend on `nodes`/`edges` and
/// return `{node_id → community_id}`.
///
/// Backend selection mirrors `graphify-py/graphify/cluster.py::_partition`:
/// Leiden is the primary (always available in Rust via `leiden-rs`), with
/// Louvain retained as a fallback that can be selected via the
/// `GRAPHIFY_CLUSTER_BACKEND=louvain` env var for debugging or quality
/// comparison.
pub(crate) fn run_partition(
    nodes: &[String],
    edges: &[(String, String, f64)],
    resolution: f64,
) -> IndexMap<String, i64> {
    // Lowercase the env value so `GRAPHIFY_CLUSTER_BACKEND=Louvain` and
    // similar capitalisations resolve to the same backend as the
    // canonical lowercase form.
    let backend = std::env::var("GRAPHIFY_CLUSTER_BACKEND")
        .ok()
        .filter(|s| !s.is_empty())
        .map_or_else(|| "leiden".to_string(), |s| s.to_ascii_lowercase());
    if !matches!(backend.as_str(), "leiden" | "louvain") {
        eprintln!(
            "[graphify] cluster: unknown GRAPHIFY_CLUSTER_BACKEND={backend:?}; \
             falling back to leiden"
        );
    }
    let raw = match backend.as_str() {
        "louvain" => crate::louvain::partition(nodes, edges, resolution),
        _ => crate::leiden::partition(nodes, edges, resolution),
    };
    // community IDs from either backend are small indices; casting
    // usize → i64 is safe for any realistic graph (community index bounded
    // by node count).
    #[allow(clippy::cast_possible_wrap)] // community IDs are small indices bounded by node count
    raw.into_iter()
        .map(|(node, cid)| (node, cid as i64))
        .collect()
}
