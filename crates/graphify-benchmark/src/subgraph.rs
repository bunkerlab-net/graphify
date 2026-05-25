//! BFS subgraph token-cost estimation for sample questions.

use serde_json::Value;

use graphify_build::Graph;

use crate::tokens::estimate_tokens;

/// Run BFS from the best-matching nodes and return the estimated token
/// count for the resulting subgraph context.
///
/// Terms come from [`graphify_serve::query_terms`], which lowercases each
/// token and keeps short non-English tokens (e.g. CJK) searchable while
/// dropping short English stop-words. Node labels are likewise lowercased
/// before the substring match, so seeding is case-insensitive. The top-3
/// scoring nodes seed the BFS; `depth` controls how many hops to expand.
/// Returns 0 when no nodes match the query terms.
#[must_use]
pub fn query_subgraph_tokens(graph: &Graph, question: &str, depth: usize) -> usize {
    let terms: Vec<String> = graphify_serve::query_terms(question);

    let mut scored: Vec<(usize, &str)> = graph
        .nodes()
        .filter_map(|(nid, data)| {
            let label = data
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            let score = terms.iter().filter(|t| label.contains(t.as_str())).count();
            if score > 0 {
                Some((score, nid.as_str()))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));

    let start_nodes: Vec<&str> = scored.iter().take(3).map(|(_, nid)| *nid).collect();
    if start_nodes.is_empty() {
        return 0;
    }

    // Build an adjacency map once (`O(|edges|)`) so BFS is `O(edges_traversed)`
    // per depth iteration rather than re-scanning every edge for every
    // frontier node.
    let mut adjacency: indexmap::IndexMap<&str, Vec<&str>> = indexmap::IndexMap::new();
    for edge in graph.edges() {
        adjacency
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        adjacency
            .entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }

    let mut visited: indexmap::IndexSet<&str> = start_nodes.iter().copied().collect();
    let mut frontier: indexmap::IndexSet<&str> = start_nodes.iter().copied().collect();
    let mut edges_seen: Vec<(&str, &str)> = Vec::new();

    for _ in 0..depth {
        let mut next_frontier: indexmap::IndexSet<&str> = indexmap::IndexSet::new();
        for &n in &frontier {
            let Some(neighbors) = adjacency.get(n) else {
                continue;
            };
            for &nb in neighbors {
                if !visited.contains(nb) {
                    next_frontier.insert(nb);
                    edges_seen.push((n, nb));
                }
            }
        }
        visited.extend(next_frontier.iter().copied());
        frontier = next_frontier;
    }

    let mut lines: Vec<String> = Vec::new();
    for nid in &visited {
        if let Some(data) = graph.node_data(nid) {
            let label = data.get("label").and_then(Value::as_str).unwrap_or(nid);
            let src = data
                .get("source_file")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let loc = data
                .get("source_location")
                .and_then(Value::as_str)
                .unwrap_or_default();
            lines.push(format!("NODE {label} src={src} loc={loc}"));
        }
    }
    for &(u, v) in &edges_seen {
        if visited.contains(u) && visited.contains(v) {
            let u_label = graph
                .node_data(u)
                .and_then(|d| d.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(u);
            let v_label = graph
                .node_data(v)
                .and_then(|d| d.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(v);
            let relation = graph
                .edge_data(u, v)
                .and_then(|d| d.get("relation"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            lines.push(format!("EDGE {u_label} --{relation}--> {v_label}"));
        }
    }

    estimate_tokens(&lines.join("\n"))
}
