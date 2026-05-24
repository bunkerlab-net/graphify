//! Community list and cohesion table renderers.
//!
//! Extracted from `lib.rs` to group navigation-hub and per-community detail
//! renderers together.  Both renderers iterate over the community list and
//! share helper imports.

use std::collections::HashMap;

use graphify_build::Graph;
use serde_json::Value;

use super::is_file_node;
use crate::safe_community_name;

/// Render the "Community Hubs (Navigation)" section.
pub(crate) fn render_nav_hubs(
    lines: &mut Vec<String>,
    non_empty: &[(i64, &Vec<&str>)],
    community_labels: &HashMap<i64, &str>,
) {
    lines.push(String::new());
    lines.push("## Community Hubs (Navigation)".to_string());
    for (cid, _) in non_empty {
        let label = community_labels
            .get(cid)
            .copied()
            .map_or_else(|| format!("Community {cid}"), ToString::to_string);
        let safe = safe_community_name(&label);
        lines.push(format!("- [[_COMMUNITY_{safe}|{label}]]"));
    }
}

/// Read-only context bundle for community-section rendering.
pub(crate) struct CommunitiesCtx<'a> {
    /// The graph being reported on.
    pub graph: &'a Graph,
    /// Ordered list of `(community_id, [node_id, ...])` pairs.
    pub communities: &'a [(i64, Vec<&'a str>)],
    /// Per-community cohesion scores keyed by community ID.
    pub cohesion_scores: &'a HashMap<i64, f64>,
    /// Human-readable labels for each community, keyed by community ID.
    pub community_labels: &'a HashMap<i64, &'a str>,
    /// Precomputed per-node degree map (see [`super::compute_degrees`]).
    pub degrees: &'a HashMap<String, usize>,
    /// Number of communities omitted from the report due to being below `min_community_size`.
    pub thin_count_summary: usize,
    /// Communities with fewer than this many real (non-file) nodes are omitted.
    pub min_community_size: usize,
}

/// Render the per-community detail blocks.
pub(crate) fn render_communities(lines: &mut Vec<String>, ctx: &CommunitiesCtx<'_>) {
    let CommunitiesCtx {
        graph,
        communities,
        cohesion_scores,
        community_labels,
        degrees,
        thin_count_summary,
        min_community_size,
    } = *ctx;
    lines.push(String::new());
    lines.push(format!(
        "## Communities ({} total, {thin_count_summary} thin omitted)",
        communities.len()
    ));
    for (cid, nodes) in communities {
        let label = community_labels
            .get(cid)
            .copied()
            .map_or_else(|| format!("Community {cid}"), ToString::to_string);
        let score = cohesion_scores.get(cid).copied().unwrap_or(0.0);
        let real_nodes: Vec<&&str> = nodes
            .iter()
            .filter(|n| !is_file_node(graph, n, degrees))
            .collect();
        if real_nodes.is_empty() || real_nodes.len() < min_community_size {
            continue;
        }
        let display: Vec<String> = real_nodes
            .iter()
            .take(8)
            .map(|n| {
                graph
                    .node_data(n)
                    .and_then(|a| a.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(n)
                    .to_string()
            })
            .collect();
        let suffix = if real_nodes.len() > 8 {
            format!(" (+{} more)", real_nodes.len() - 8)
        } else {
            String::new()
        };
        lines.push(String::new());
        lines.push(format!("### Community {cid} - \"{label}\""));
        lines.push(format!("Cohesion: {score:.2}"));
        lines.push(format!(
            "Nodes ({}): {}{}",
            real_nodes.len(),
            display.join(", "),
            suffix
        ));
    }
}
