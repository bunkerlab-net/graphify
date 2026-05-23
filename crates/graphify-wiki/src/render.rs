//! Markdown renderers for community, god-node, and index articles.

use std::cmp::Reverse;
use std::collections::HashMap;

use indexmap::{IndexMap, IndexSet};

use graphify_build::Graph;

use crate::types::GodNodeData;
use crate::util::{audit_trail_lines, cross_community_links, neighbors_of};

/// Read-only inputs for [`community_article`].
pub(crate) struct CommunityArticleArgs<'a> {
    pub graph: &'a Graph,
    pub cid: i64,
    pub nodes: &'a [String],
    pub label: &'a str,
    pub labels: &'a IndexMap<i64, String>,
    pub cohesion: Option<f64>,
    pub node_community: &'a HashMap<String, i64>,
    pub deg_map: &'a HashMap<&'a str, usize>,
}

/// Render one community article as a Markdown string.
///
/// Builds, in order: the title, a metadata blockquote, a "Key Concepts"
/// list of the top-25 nodes by degree, a "Relationships" list of the most
/// linked sibling communities, an optional "Source Files" listing, and the
/// "Audit Trail" confidence breakdown.
///
/// `clippy::too_many_lines` is suppressed: the function is a single sequential
/// markdown emission where each phase reads earlier-computed locals; splitting
/// fragments the linear flow without isolating reusable pieces.
#[must_use]
#[allow(clippy::too_many_lines)]
pub(crate) fn community_article(args: &CommunityArticleArgs<'_>) -> String {
    let CommunityArticleArgs {
        graph,
        cid,
        nodes,
        label,
        labels,
        cohesion,
        node_community,
        deg_map,
    } = *args;
    let mut sorted_nodes: Vec<&String> = nodes.iter().collect();
    sorted_nodes.sort_by(|a, b| {
        let da = deg_map.get(a.as_str()).copied().unwrap_or(0);
        let db = deg_map.get(b.as_str()).copied().unwrap_or(0);
        db.cmp(&da)
    });
    let top_nodes: Vec<&String> = sorted_nodes.iter().copied().take(25).collect();

    let cross = cross_community_links(graph, nodes, cid, labels, node_community);

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

/// Render one god-node article as a Markdown string.
///
/// Lists the node's neighbors grouped by edge relation, sorted within each
/// group by neighbor degree (descending). Truncates each relation group at
/// 20 entries to keep the document readable.
#[must_use]
pub(crate) fn god_node_article(
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

/// Render the `index.md` content for the wiki.
///
/// Lists communities (sorted by size, largest first) and any god-node
/// articles, followed by a generator footer. Acts as the agent's entry
/// point into the wiki.
#[must_use]
pub(crate) fn index_md(
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
