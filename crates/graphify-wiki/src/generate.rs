//! Top-level `to_wiki` driver: filters stale node IDs, then writes the
//! community articles, god-node articles, and `index.md`.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use indexmap::{IndexMap, IndexSet};

use graphify_build::Graph;

use crate::error::WikiError;
use crate::render::{community_article, god_node_article, index_md};
use crate::types::GodNodeData;
use crate::util::{build_degree_map, safe_filename};

/// Generate a Wikipedia-style wiki from the graph.
///
/// Writes the following files under `output_dir`:
/// - `index.md` — agent entry point, catalog of all articles
/// - `<CommunityName>.md` — one article per community
/// - `<GodNodeLabel>.md` — one article per god node
///
/// Before writing, removes every existing `*.md` file in `output_dir` so
/// articles deleted in a prior run do not linger. Returns the number of
/// articles written (excluding `index.md`).
///
/// # Errors
///
/// - [`WikiError::EmptyCommunities`] if `communities` is empty.
/// - [`WikiError::AllStale`] if every node ID in every community is absent
///   from the graph after stale-ID filtering.
/// - [`WikiError::Io`] on any filesystem error.
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
        let _ = writeln!(
            std::io::stderr(),
            "wiki: dropped {dropped} stale node ID(s) not in graph ({remaining} communities remaining)",
        );
    }

    if filtered.is_empty() {
        return Err(WikiError::AllStale);
    }

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

    let node_community: HashMap<String, i64> = filtered
        .iter()
        .flat_map(|(&cid, nodes)| nodes.iter().map(move |n| (n.clone(), cid)))
        .collect();

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

    for node_data in god_nodes_data {
        if graph.contains_node(&node_data.id) {
            let article = god_node_article(graph, &node_data.id, labels, &node_community, &deg_map);
            let slug = unique_slug(safe_filename(&node_data.label));
            let path: PathBuf = output_dir.join(format!("{slug}.md"));
            std::fs::write(&path, article.as_bytes())?;
            count += 1;
        }
    }

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
