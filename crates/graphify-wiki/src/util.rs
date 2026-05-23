//! Filename and graph-walking helpers shared by the article renderers.

use std::cmp::Reverse;
use std::collections::HashMap;

use indexmap::IndexMap;

use graphify_build::Graph;

/// Make a label safe for use as a filename across platforms.
///
/// Substitutes characters that Windows reserves in filenames
/// (`< > : " / \\ | ? *`) and strips trailing dots/spaces, also reserved.
/// Falls back to `"unnamed"` for empty results and caps length at 200 chars.
#[must_use]
pub(crate) fn safe_filename(name: &str) -> String {
    let s = name.replace('/', "-").replace(' ', "_").replace(':', "-");
    let s: String = s
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim_matches(|c| c == '.' || c == ' ').to_string();
    if s.is_empty() {
        "unnamed".to_string()
    } else if s.len() > 200 {
        s[..200].to_string()
    } else {
        s
    }
}

/// Compute per-node degree (number of incident edges, undirected).
///
/// Self-loops contribute one to the source's degree only, matching the Python
/// reference's semantics.
pub(crate) fn build_degree_map(graph: &Graph) -> HashMap<&str, usize> {
    let mut deg: HashMap<&str, usize> = HashMap::new();
    for edge in graph.edges() {
        *deg.entry(edge.source.as_str()).or_insert(0) += 1;
        if edge.source != edge.target {
            *deg.entry(edge.target.as_str()).or_insert(0) += 1;
        }
    }
    deg
}

/// Collect neighbors of `nid` (both directions for undirected graph).
pub(crate) fn neighbors_of<'g>(graph: &'g Graph, nid: &str) -> Vec<&'g str> {
    let mut out: Vec<&'g str> = Vec::new();
    for edge in graph.edges() {
        if edge.source == nid {
            out.push(edge.target.as_str());
        } else if edge.target == nid {
            out.push(edge.source.as_str());
        }
    }
    out
}

/// Return `(community_label, edge_count)` pairs for cross-community
/// connections, sorted descending by edge count.
pub(crate) fn cross_community_links(
    graph: &Graph,
    nodes: &[String],
    own_cid: i64,
    labels: &IndexMap<i64, String>,
    node_community: &HashMap<String, i64>,
) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for nid in nodes {
        for neighbor in neighbors_of(graph, nid) {
            if let Some(&ncid) = node_community.get(neighbor)
                && ncid != own_cid
            {
                let label = labels
                    .get(&ncid)
                    .cloned()
                    .unwrap_or_else(|| format!("Community {ncid}"));
                *counts.entry(label).or_insert(0) += 1;
            }
        }
    }
    let mut result: Vec<(String, usize)> = counts.into_iter().collect();
    result.sort_by_key(|b| Reverse(b.1));
    result
}

/// Render the audit trail lines for a community article.
///
/// Produces three lines, one per confidence level (`EXTRACTED`, `INFERRED`,
/// `AMBIGUOUS`), each with a percentage rounded the same way Python's
/// built-in `round()` rounds — half-up via integer arithmetic.
pub(crate) fn audit_trail_lines(
    conf_counts: &HashMap<String, usize>,
    total_edges: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    for conf in &["EXTRACTED", "INFERRED", "AMBIGUOUS"] {
        let n = conf_counts.get(*conf).copied().unwrap_or(0);
        let pct = (n * 100 + total_edges / 2) / total_edges;
        out.push(format!("- {conf}: {n} ({pct}%)"));
    }
    out
}
