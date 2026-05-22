//! Wiki/Obsidian vault export.
//!
//! Ports `graphify-py/graphify/wiki.py`. Generates `.md` files per community
//! and per god node, plus an `index.md` entry point.

use std::{
    cmp::Reverse,
    collections::HashMap,
    io::Write as _,
    path::{Path, PathBuf},
};

use indexmap::{IndexMap, IndexSet};
use thiserror::Error;

use graphify_build::Graph;

/// Errors produced by [`to_wiki`].
#[derive(Debug, Error)]
pub enum WikiError {
    #[error(
        "communities dict is empty — refusing to clear wiki/. \
         Run `graphify extract .` or `graphify cluster-only .` first."
    )]
    EmptyCommunities,

    #[error(
        "all community node IDs are stale — none exist in the graph. \
         Re-run `graphify extract .` to regenerate .graphify_analysis.json."
    )]
    AllStale,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Make a label safe for use as a filename across platforms.
///
/// Substitutes characters that Windows reserves in filenames
/// (`< > : " / \\ | ? *`) and strips trailing dots/spaces, also reserved.
/// Falls back to `"unnamed"` for empty results and caps length at 200 chars.
#[must_use]
fn safe_filename(name: &str) -> String {
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
fn build_degree_map(graph: &Graph) -> HashMap<&str, usize> {
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
fn neighbors_of<'g>(graph: &'g Graph, nid: &str) -> Vec<&'g str> {
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
/// connections sorted descending.
fn cross_community_links(
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
fn audit_trail_lines(conf_counts: &HashMap<String, usize>, total_edges: usize) -> Vec<String> {
    let mut out = Vec::new();
    for conf in &["EXTRACTED", "INFERRED", "AMBIGUOUS"] {
        let n = conf_counts.get(*conf).copied().unwrap_or(0);
        // Integer rounding that matches Python's round(): add half of divisor before
        // integer division. For percentages this is: (n*100 + total/2) / total.
        // Both n and total_edges are at most edge-count sized; no precision loss.
        let pct = (n * 100 + total_edges / 2) / total_edges;
        out.push(format!("- {conf}: {n} ({pct}%)"));
    }
    out
}

/// Render one community article.
///
/// Suppressed: `clippy::too_many_arguments` — this is a pure rendering
/// function that needs all contextual parameters; splitting it into a struct
/// would add indirection with no benefit.
/// Suppressed: `clippy::too_many_lines` — the function is long because it
/// builds every section of the Markdown article inline; extracting helpers
/// would obscure the 1:1 mapping with the Python reference.
#[must_use]
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
// reason: pure rendering function — all args are required context; splitting adds no benefit
fn community_article(
    graph: &Graph,
    cid: i64,
    nodes: &[String],
    label: &str,
    labels: &IndexMap<i64, String>,
    cohesion: Option<f64>,
    node_community: &HashMap<String, i64>,
    deg_map: &HashMap<&str, usize>,
) -> String {
    // Top 25 nodes by degree, descending.
    let mut sorted_nodes: Vec<&String> = nodes.iter().collect();
    sorted_nodes.sort_by(|a, b| {
        let da = deg_map.get(a.as_str()).copied().unwrap_or(0);
        let db = deg_map.get(b.as_str()).copied().unwrap_or(0);
        db.cmp(&da)
    });
    let top_nodes: Vec<&String> = sorted_nodes.iter().copied().take(25).collect();

    let cross = cross_community_links(graph, nodes, cid, labels, node_community);

    // Edge confidence breakdown.
    let mut conf_counts: HashMap<String, usize> = HashMap::new();
    for nid in nodes {
        for neighbor in neighbors_of(graph, nid) {
            let ed = graph.edge_data(nid, neighbor);
            let conf = ed
                .and_then(|m| m.get("confidence"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("EXTRACTED")
                .to_string();
            *conf_counts.entry(conf).or_insert(0) += 1;
        }
    }
    let total_edges = conf_counts.values().sum::<usize>().max(1);

    // Collect unique source files.
    let mut sources: IndexSet<String> = IndexSet::new();
    for nid in nodes {
        if let Some(attrs) = graph.node_data(nid)
            && let Some(serde_json::Value::String(sf)) = attrs.get("source_file")
            && !sf.is_empty()
        {
            sources.insert(sf.clone());
        }
    }
    let mut sources_sorted: Vec<String> = sources.into_iter().collect();
    sources_sorted.sort();

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("# {label}"));
    lines.push(String::new());

    let meta = if let Some(c) = cohesion {
        format!("{} nodes · cohesion {c:.2}", nodes.len())
    } else {
        format!("{} nodes", nodes.len())
    };
    lines.push(format!("> {meta}"));
    lines.push(String::new());

    lines.push("## Key Concepts".to_string());
    lines.push(String::new());
    for nid in &top_nodes {
        let attrs = graph.node_data(nid);
        let node_label = attrs
            .and_then(|m| m.get("label"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(nid.as_str());
        let src = attrs
            .and_then(|m| m.get("source_file"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let degree = deg_map.get(nid.as_str()).copied().unwrap_or(0);
        let src_str = if src.is_empty() {
            String::new()
        } else {
            format!(" — `{src}`")
        };
        lines.push(format!(
            "- **{node_label}** ({degree} connections){src_str}"
        ));
    }
    let remaining = nodes.len().saturating_sub(top_nodes.len());
    if remaining > 0 {
        lines.push(format!(
            "- *... and {remaining} more nodes in this community*"
        ));
    }
    lines.push(String::new());

    lines.push("## Relationships".to_string());
    lines.push(String::new());
    if cross.is_empty() {
        lines.push("- No strong cross-community connections detected".to_string());
    } else {
        for (other_label, count) in cross.iter().take(12) {
            lines.push(format!("- [[{other_label}]] ({count} shared connections)"));
        }
    }
    lines.push(String::new());

    if !sources_sorted.is_empty() {
        lines.push("## Source Files".to_string());
        lines.push(String::new());
        for src in sources_sorted.iter().take(20) {
            lines.push(format!("- `{src}`"));
        }
        lines.push(String::new());
    }

    lines.push("## Audit Trail".to_string());
    lines.push(String::new());
    lines.extend(audit_trail_lines(&conf_counts, total_edges));
    lines.push(String::new());

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("*Part of the graphify knowledge wiki. See [[index]] to navigate.*".to_string());

    lines.join("\n")
}

/// Render one god-node article.
#[must_use]
fn god_node_article(
    graph: &Graph,
    nid: &str,
    labels: &IndexMap<i64, String>,
    node_community: &HashMap<String, i64>,
    deg_map: &HashMap<&str, usize>,
) -> String {
    let attrs = graph.node_data(nid);
    let node_label = attrs
        .and_then(|m| m.get("label"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(nid);
    let src = attrs
        .and_then(|m| m.get("source_file"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let degree = deg_map.get(nid).copied().unwrap_or(0);
    let community_name: Option<String> = node_community.get(nid).map(|cid| {
        labels
            .get(cid)
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"))
    });

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("# {node_label}"));
    lines.push(String::new());
    lines.push(format!("> God node · {degree} connections · `{src}`"));
    lines.push(String::new());

    if let Some(ref cn) = community_name {
        lines.push(format!("**Community:** [[{cn}]]"));
        lines.push(String::new());
    }

    // Group neighbors by relation type; sort neighbors by degree descending.
    let mut neighbors: Vec<&str> = neighbors_of(graph, nid);
    neighbors.sort_by(|a, b| {
        let da = deg_map.get(a).copied().unwrap_or(0);
        let db = deg_map.get(b).copied().unwrap_or(0);
        db.cmp(&da)
    });

    let mut by_relation: IndexMap<String, Vec<String>> = IndexMap::new();
    for neighbor in neighbors {
        let nd = graph.node_data(neighbor);
        let ed = graph.edge_data(nid, neighbor);
        let rel = ed
            .and_then(|m| m.get("relation"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("related")
            .to_string();
        let neighbor_label = nd
            .and_then(|m| m.get("label"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(neighbor);
        let conf = ed
            .and_then(|m| m.get("confidence"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let conf_str = if conf.is_empty() {
            String::new()
        } else {
            format!(" `{conf}`")
        };
        by_relation
            .entry(rel)
            .or_default()
            .push(format!("[[{neighbor_label}]]{conf_str}"));
    }

    lines.push("## Connections by Relation".to_string());
    lines.push(String::new());
    // Sort by relation name.
    let mut rel_entries: Vec<(String, Vec<String>)> = by_relation.into_iter().collect();
    rel_entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, targets) in rel_entries {
        lines.push(format!("### {rel}"));
        for t in targets.iter().take(20) {
            lines.push(format!("- {t}"));
        }
        lines.push(String::new());
    }

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("*Part of the graphify knowledge wiki. See [[index]] to navigate.*".to_string());

    lines.join("\n")
}

/// Render the `index.md` content.
#[must_use]
fn index_md(
    communities: &IndexMap<i64, Vec<String>>,
    labels: &IndexMap<i64, String>,
    god_nodes_data: &[GodNodeData],
    total_nodes: usize,
    total_edges: usize,
) -> String {
    let mut lines: Vec<String> = vec![
        "# Knowledge Graph Index".to_string(),
        String::new(),
        "> Auto-generated by graphify. Start here — read community articles for context, then drill into god nodes for detail.".to_string(),
        String::new(),
        format!(
            "**{total_nodes} nodes · {total_edges} edges · {} communities**",
            communities.len()
        ),
        String::new(),
        "---".to_string(),
        String::new(),
        "## Communities".to_string(),
        "(sorted by size, largest first)".to_string(),
        String::new(),
    ];

    // Sort communities by descending node count.
    let mut sorted_cids: Vec<i64> = communities.keys().copied().collect();
    sorted_cids.sort_by_key(|cid| Reverse(communities.get(cid).map_or(0, Vec::len)));
    for cid in sorted_cids {
        let nodes = &communities[&cid];
        let label = labels
            .get(&cid)
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        lines.push(format!("- [[{label}]] — {} nodes", nodes.len()));
    }
    lines.push(String::new());

    if !god_nodes_data.is_empty() {
        lines.push("## God Nodes".to_string());
        lines.push("(most connected concepts — the load-bearing abstractions)".to_string());
        lines.push(String::new());
        for node in god_nodes_data {
            lines.push(format!(
                "- [[{}]] — {} connections",
                node.label, node.degree
            ));
        }
        lines.push(String::new());
    }

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("*Generated by [graphify](https://github.com/safishamsi/graphify)*".to_string());

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Structured data for a god node passed to [`to_wiki`].
#[derive(Debug, Clone)]
pub struct GodNodeData {
    /// Node ID in the graph.
    pub id: String,
    /// Display label for the article title.
    pub label: String,
    /// Pre-computed connection degree (used in index listing).
    pub degree: usize,
}

/// Generate a Wikipedia-style wiki from the graph.
///
/// Writes:
/// - `index.md` — agent entry point, catalog of all articles
/// - `<CommunityName>.md` — one article per community
/// - `<GodNodeLabel>.md` — one article per god node
///
/// Returns the number of articles written (excluding `index.md`).
///
/// # Errors
///
/// Returns [`WikiError::EmptyCommunities`] if `communities` is empty.
/// Returns [`WikiError::AllStale`] if every node ID in every community is
/// absent from the graph after stale-ID filtering.
/// Returns [`WikiError::Io`] on any filesystem error.
///
/// Suppressed: `clippy::too_many_lines` — the function encodes the complete
/// Python `to_wiki` logic in a single place; extracting sub-helpers would
/// obscure the 1:1 port mapping.
#[allow(clippy::too_many_lines)]
// reason: encodes the full Python to_wiki() pipeline; splitting obscures the 1:1 port
pub fn to_wiki(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_dir: &Path,
    community_labels: Option<&IndexMap<i64, String>>,
    cohesion: Option<&IndexMap<i64, f64>>,
    god_nodes_data: Option<&[GodNodeData]>,
) -> Result<usize, WikiError> {
    std::fs::create_dir_all(output_dir)?;

    if communities.is_empty() {
        return Err(WikiError::EmptyCommunities);
    }

    // Filter stale node IDs.
    let g_nodes: IndexSet<&str> = graph.nodes().map(|(id, _)| id.as_str()).collect();
    let orig_total: usize = communities.values().map(Vec::len).sum();
    let filtered: IndexMap<i64, Vec<String>> = communities
        .iter()
        .filter_map(|(&cid, nodes)| {
            let live: Vec<String> = nodes
                .iter()
                .filter(|n| g_nodes.contains(n.as_str()))
                .cloned()
                .collect();
            if live.is_empty() {
                None
            } else {
                Some((cid, live))
            }
        })
        .collect();
    let kept_total: usize = filtered.values().map(Vec::len).sum();

    if kept_total < orig_total {
        let dropped = orig_total - kept_total;
        let remaining = filtered.len();
        // Print to stderr, matching Python's message format.
        let _ = writeln!(
            std::io::stderr(),
            "wiki: dropped {dropped} stale node ID(s) not in graph ({remaining} communities remaining)",
        );
    }

    if filtered.is_empty() {
        return Err(WikiError::AllStale);
    }

    // Clear stale .md files from previous runs.
    for entry in std::fs::read_dir(output_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            std::fs::remove_file(&path)?;
        }
    }

    let default_labels: IndexMap<i64, String> = filtered
        .keys()
        .map(|&cid| (cid, format!("Community {cid}")))
        .collect();
    let labels: &IndexMap<i64, String> = community_labels.unwrap_or(&default_labels);
    let empty_cohesion = IndexMap::new();
    let cohesion = cohesion.unwrap_or(&empty_cohesion);
    let no_gods: &[GodNodeData] = &[];
    let god_nodes_data = god_nodes_data.unwrap_or(no_gods);

    // Build node→community lookup.
    let node_community: HashMap<String, i64> = filtered
        .iter()
        .flat_map(|(&cid, nodes)| nodes.iter().map(move |n| (n.clone(), cid)))
        .collect();

    // Pre-compute degree map.
    let deg_map = build_degree_map(graph);

    let mut count = 0usize;
    let mut used_slugs: IndexSet<String> = IndexSet::new();

    let mut unique_slug = |base: String| -> String {
        let mut slug = base.clone();
        let mut n = 2usize;
        while used_slugs.contains(&slug) {
            slug = format!("{base}_{n}");
            n += 1;
        }
        used_slugs.insert(slug.clone());
        slug
    };

    // Community articles.
    for (&cid, nodes) in &filtered {
        let label = labels
            .get(&cid)
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        let article = community_article(
            graph,
            cid,
            nodes,
            &label,
            labels,
            cohesion.get(&cid).copied(),
            &node_community,
            &deg_map,
        );
        let slug = unique_slug(safe_filename(&label));
        let path: PathBuf = output_dir.join(format!("{slug}.md"));
        std::fs::write(&path, article.as_bytes())?;
        count += 1;
    }

    // God node articles.
    for node_data in god_nodes_data {
        if graph.contains_node(&node_data.id) {
            let article = god_node_article(graph, &node_data.id, labels, &node_community, &deg_map);
            let slug = unique_slug(safe_filename(&node_data.label));
            let path: PathBuf = output_dir.join(format!("{slug}.md"));
            std::fs::write(&path, article.as_bytes())?;
            count += 1;
        }
    }

    // Index.
    let index = index_md(
        &filtered,
        labels,
        god_nodes_data,
        graph.node_count(),
        graph.edge_count(),
    );
    let index_path: PathBuf = output_dir.join("index.md");
    std::fs::write(&index_path, index.as_bytes())?;

    Ok(count)
}
