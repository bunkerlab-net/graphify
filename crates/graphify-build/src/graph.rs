//! Graph data structure modelling ``NetworkX`` `Graph` / `DiGraph` /
//! `MultiGraph` / `MultiDiGraph` semantics.

use indexmap::IndexMap;
use serde_json::Value;

/// Variants of the graph type, matching ``NetworkX``.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphKind {
    /// Undirected, no parallel edges (``NetworkX`` `Graph`).
    #[default]
    Graph,
    /// Directed, no parallel edges (``NetworkX`` `DiGraph`).
    DiGraph,
    /// Undirected, parallel edges allowed (``NetworkX`` `MultiGraph`).
    MultiGraph,
    /// Directed, parallel edges allowed (``NetworkX`` `MultiDiGraph`).
    MultiDiGraph,
}

impl GraphKind {
    /// Returns `true` if this graph kind models directed edges.
    #[must_use]
    pub fn is_directed(self) -> bool {
        matches!(self, GraphKind::DiGraph | GraphKind::MultiDiGraph)
    }
    /// Returns `true` if this graph kind allows parallel edges between the same node pair.
    #[must_use]
    pub fn is_multi(self) -> bool {
        matches!(self, GraphKind::MultiGraph | GraphKind::MultiDiGraph)
    }
}

/// One edge entry. Mirrors a ``NetworkX`` `(u, v, attrs)` tuple.
#[derive(Debug, Clone, Default)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub attrs: IndexMap<String, Value>,
}

/// Graph container with ``NetworkX``-equivalent semantics for the operations
/// graphify exercises. Iteration order is insertion order (matches Python 3.7+).
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub kind: GraphKind,
    /// Node id → attributes.
    pub node_map: IndexMap<String, IndexMap<String, Value>>,
    /// All edges in insertion order.
    pub edge_list: Vec<Edge>,
    /// Graph-level attributes (e.g. `hyperedges`).
    pub graph_attrs: IndexMap<String, Value>,
}

impl Graph {
    /// Creates an empty graph of the given `kind`.
    #[must_use]
    pub fn new(kind: GraphKind) -> Self {
        Self {
            kind,
            node_map: IndexMap::new(),
            edge_list: Vec::new(),
            graph_attrs: IndexMap::new(),
        }
    }

    /// Inserts or replaces the node with the given `id`, overwriting any existing attribute map.
    pub fn add_node(&mut self, id: &str, attrs: IndexMap<String, Value>) {
        // `NetworkX` add_node is idempotent — repeated calls *overwrite* the
        // attribute map with the new one. We replicate that exactly.
        self.node_map.insert(id.to_string(), attrs);
    }

    /// Add an edge. For non-multi graphs the most recent `attrs` wins for the
    /// matching `(src, tgt)` pair (or its reverse on undirected). For multi
    /// graphs every call appends a new parallel edge.
    ///
    /// O(N) per call because non-multi graphs scan `edge_list` for a
    /// dedup match. For bulk insertion (e.g. building a graph from an
    /// extraction dict) prefer [`Self::bulk_add_edges`] which uses a
    /// `HashMap` to amortise dedup to `O(N+E)`.
    pub fn add_edge(&mut self, src: &str, tgt: &str, attrs: IndexMap<String, Value>) {
        if !self.kind.is_multi() {
            let directed = self.kind.is_directed();
            for edge in &mut self.edge_list {
                let matches = if directed {
                    edge.source == src && edge.target == tgt
                } else {
                    (edge.source == src && edge.target == tgt)
                        || (edge.source == tgt && edge.target == src)
                };
                if matches {
                    edge.attrs = attrs;
                    return;
                }
            }
        }
        self.edge_list.push(Edge {
            source: src.to_string(),
            target: tgt.to_string(),
            attrs,
        });
    }

    /// Bulk insert a batch of edges, deduplicating in `O(N + E)` instead of
    /// the per-call `add_edge`'s `O(N²)`. Preserves "last-attrs-wins"
    /// semantics for non-multi graphs (matches `NetworkX`).
    ///
    /// The single-call `add_edge` is fine for hand-written graphs of any
    /// size; this method exists specifically to make `build_from_json`
    /// linear on large extraction inputs (36k+ edges).
    pub fn bulk_add_edges<I>(&mut self, edges: I)
    where
        I: IntoIterator<Item = (String, String, IndexMap<String, Value>)>,
    {
        if self.kind.is_multi() {
            // No dedup possible — every call is a fresh parallel edge.
            for (src, tgt, attrs) in edges {
                self.edge_list.push(Edge {
                    source: src,
                    target: tgt,
                    attrs,
                });
            }
            return;
        }

        // Build an index from canonical (src, tgt) pair → existing
        // edge_list position so we can replace attrs in O(1) when a
        // duplicate arrives. Seeded from any edges already on the graph.
        let directed = self.kind.is_directed();
        let canonical = |a: &str, b: &str| -> (String, String) {
            if directed || a <= b {
                (a.to_string(), b.to_string())
            } else {
                (b.to_string(), a.to_string())
            }
        };
        let mut index: std::collections::HashMap<(String, String), usize> =
            std::collections::HashMap::with_capacity(self.edge_list.len());
        for (idx, edge) in self.edge_list.iter().enumerate() {
            index.insert(canonical(&edge.source, &edge.target), idx);
        }

        for (src, tgt, attrs) in edges {
            let key = canonical(&src, &tgt);
            if let Some(&existing) = index.get(&key) {
                self.edge_list[existing].attrs = attrs;
            } else {
                let idx = self.edge_list.len();
                self.edge_list.push(Edge {
                    source: src,
                    target: tgt,
                    attrs,
                });
                index.insert(key, idx);
            }
        }
    }

    /// Returns `true` if a node with the given `id` exists in the graph.
    #[must_use]
    pub fn contains_node(&self, id: &str) -> bool {
        self.node_map.contains_key(id)
    }

    /// Returns a shared reference to the attribute map for node `id`, or `None` if absent.
    #[must_use]
    pub fn node_data(&self, id: &str) -> Option<&IndexMap<String, Value>> {
        self.node_map.get(id)
    }

    /// Returns a mutable reference to the attribute map for node `id`, or `None` if absent.
    pub fn node_data_mut(&mut self, id: &str) -> Option<&mut IndexMap<String, Value>> {
        self.node_map.get_mut(id)
    }

    /// Returns an iterator over `(id, attrs)` pairs in insertion order.
    pub fn nodes(&self) -> impl Iterator<Item = (&String, &IndexMap<String, Value>)> {
        self.node_map.iter()
    }

    /// Returns a mutable iterator over `(id, attrs)` pairs in insertion order.
    pub fn nodes_mut(&mut self) -> impl Iterator<Item = (&String, &mut IndexMap<String, Value>)> {
        self.node_map.iter_mut()
    }

    /// Returns the number of nodes in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.node_map.len()
    }

    /// Returns an iterator over all edges in insertion order.
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edge_list.iter()
    }

    /// Returns the number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edge_list.len()
    }

    /// First edge attribute dict for `(u, v)`. Mirrors ``NetworkX`` `G[u][v]`
    /// semantics with `MultiGraph` tolerance.
    #[must_use]
    pub fn edge_data(&self, u: &str, v: &str) -> Option<&IndexMap<String, Value>> {
        let directed = self.kind.is_directed();
        for edge in &self.edge_list {
            let m = if directed {
                edge.source == u && edge.target == v
            } else {
                (edge.source == u && edge.target == v) || (edge.source == v && edge.target == u)
            };
            if m {
                return Some(&edge.attrs);
            }
        }
        None
    }

    /// Every edge attribute dict for `(u, v)`.
    #[must_use]
    pub fn edge_datas(&self, u: &str, v: &str) -> Vec<&IndexMap<String, Value>> {
        let directed = self.kind.is_directed();
        let mut out = Vec::new();
        for edge in &self.edge_list {
            let m = if directed {
                edge.source == u && edge.target == v
            } else {
                (edge.source == u && edge.target == v) || (edge.source == v && edge.target == u)
            };
            if m {
                out.push(&edge.attrs);
            }
        }
        out
    }

    /// Relabel nodes in place via `mapping` (`old_id` → `new_id`).
    pub fn relabel_nodes(&mut self, mapping: &IndexMap<String, String>) {
        let mut new_nodes: IndexMap<String, IndexMap<String, Value>> = IndexMap::new();
        for (id, attrs) in &self.node_map {
            let new_id = mapping.get(id).cloned().unwrap_or_else(|| id.clone());
            new_nodes.insert(new_id, attrs.clone());
        }
        self.node_map = new_nodes;
        for edge in &mut self.edge_list {
            if let Some(new) = mapping.get(&edge.source) {
                edge.source.clone_from(new);
            }
            if let Some(new) = mapping.get(&edge.target) {
                edge.target.clone_from(new);
            }
        }
    }

    /// Removes all nodes in `ids` and any edges incident to them.
    pub fn remove_nodes_from<'a, I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = &'a str>,
    {
        let removed: indexmap::IndexSet<String> = ids.into_iter().map(String::from).collect();
        for id in &removed {
            self.node_map.shift_remove(id);
        }
        self.edge_list
            .retain(|e| !removed.contains(&e.source) && !removed.contains(&e.target));
    }

    /// Removes all edges matching the given `(source, target)` pairs.
    pub fn remove_edges_from<'a, I>(&mut self, pairs: I)
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let directed = self.kind.is_directed();
        let pairs: Vec<(String, String)> = pairs
            .into_iter()
            .map(|(u, v)| (u.to_string(), v.to_string()))
            .collect();
        self.edge_list.retain(|edge| {
            !pairs.iter().any(|(u, v)| {
                if directed {
                    edge.source == *u && edge.target == *v
                } else {
                    (edge.source == *u && edge.target == *v)
                        || (edge.source == *v && edge.target == *u)
                }
            })
        });
    }
}
