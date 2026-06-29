//! Top-level `to_wiki` driver: filters stale node IDs, then writes the
//! community articles, god-node articles, and `index.md`.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

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

    let filtered = filter_stale_communities(graph, communities);
    if filtered.is_empty() {
        return Err(WikiError::AllStale);
    }
    clear_existing_md_files(output_dir)?;

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

    // First pass: assign every article its slug before rendering any body, so the
    // bodies can link to one another via the resolver (#1444). A link's target is
    // the on-disk slug, which differs from the label, so it must be known up front.
    let mut used_slugs: IndexSet<String> = IndexSet::new();
    let mut resolver: HashMap<String, String> = HashMap::new();
    resolver.insert("index".to_string(), "index".to_string());
    // Parity dispute (CodeRabbit): `index` is reserved in `resolver` only, NOT in
    // `used_slugs` — matching graphify-py exactly (wiki.py: `resolver = {"index":
    // "index"}` with an empty `used_slugs`). An article literally named "index"
    // reuses the slug in both implementations; reserving it here would diverge
    // from byte-identical wiki output, so we keep graphify-py's behaviour.

    let mut community_slugs: IndexMap<i64, String> = IndexMap::new();
    for &cid in filtered.keys() {
        let label = labels
            .get(&cid)
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        let slug = make_unique_slug(&safe_filename(&label), &mut used_slugs);
        community_slugs.insert(cid, slug.clone());
        // Parity dispute (CodeRabbit): the resolver is keyed by display label,
        // mirroring graphify-py `resolver.setdefault(label, slug)`. Duplicate
        // titles collapse to the first slug in both; keying by node id instead
        // would diverge from graphify-py's byte-identical links.
        resolver.entry(label).or_insert(slug);
    }
    let mut god_articles: Vec<(String, String)> = Vec::new(); // (node_id, slug)
    for node_data in god_nodes_data {
        if graph.contains_node(&node_data.id) {
            let slug = make_unique_slug(&safe_filename(&node_data.label), &mut used_slugs);
            resolver
                .entry(node_data.label.clone())
                .or_insert(slug.clone());
            god_articles.push((node_data.id.clone(), slug));
        }
    }

    // Second pass: render and write each article with the full resolver in hand.
    let mut count = 0usize;
    let wiki_ctx = WikiCtx {
        graph,
        labels,
        node_community: &node_community,
        deg_map: &deg_map,
        resolver: &resolver,
        output_dir,
    };
    count += write_community_articles(&wiki_ctx, &filtered, cohesion, &community_slugs)?;
    count += write_god_node_articles(&wiki_ctx, &god_articles)?;

    let index = index_md(
        &filtered,
        labels,
        god_nodes_data,
        graph.node_count(),
        graph.edge_count(),
        &resolver,
    );
    std::fs::write(output_dir.join("index.md"), index.as_bytes())?;

    Ok(count)
}

/// Drop community members whose node IDs are no longer in the graph, then log
/// stale-ID drops to stderr. Returns the live-only community map.
fn filter_stale_communities(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
) -> IndexMap<i64, Vec<String>> {
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
    filtered
}

/// Remove every `*.md` file in `output_dir` so the next run starts clean.
fn clear_existing_md_files(output_dir: &Path) -> Result<(), WikiError> {
    for entry in std::fs::read_dir(output_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Generate a fresh, deduplicated filename slug, folding case in the collision
/// check so two labels differing only by case (`Parser` vs `parser`) get distinct
/// files on case-insensitive filesystems while keeping the original-case slug
/// (#1453).
fn make_unique_slug(base: &str, used_slugs: &mut IndexSet<String>) -> String {
    let mut slug = base.to_string();
    let mut n = 2usize;
    while used_slugs.contains(&slug.to_lowercase()) {
        slug = format!("{base}_{n}");
        n += 1;
    }
    used_slugs.insert(slug.to_lowercase());
    slug
}

/// Read-only context shared by every per-article writer.
struct WikiCtx<'a> {
    graph: &'a Graph,
    labels: &'a IndexMap<i64, String>,
    node_community: &'a HashMap<String, i64>,
    deg_map: &'a HashMap<&'a str, usize>,
    resolver: &'a HashMap<String, String>,
    output_dir: &'a Path,
}

fn write_community_articles(
    ctx: &WikiCtx<'_>,
    filtered: &IndexMap<i64, Vec<String>>,
    cohesion: &IndexMap<i64, f64>,
    community_slugs: &IndexMap<i64, String>,
) -> Result<usize, WikiError> {
    let mut count = 0usize;
    for (&cid, nodes) in filtered {
        let label = ctx
            .labels
            .get(&cid)
            .cloned()
            .unwrap_or_else(|| format!("Community {cid}"));
        let article = community_article(&crate::render::CommunityArticleArgs {
            graph: ctx.graph,
            cid,
            nodes,
            label: &label,
            labels: ctx.labels,
            cohesion: cohesion.get(&cid).copied(),
            node_community: ctx.node_community,
            deg_map: ctx.deg_map,
            resolver: ctx.resolver,
        });
        let slug = &community_slugs[&cid];
        std::fs::write(
            ctx.output_dir.join(format!("{slug}.md")),
            article.as_bytes(),
        )?;
        count += 1;
    }
    Ok(count)
}

fn write_god_node_articles(
    ctx: &WikiCtx<'_>,
    god_articles: &[(String, String)],
) -> Result<usize, WikiError> {
    for (nid, slug) in god_articles {
        let article = god_node_article(
            ctx.graph,
            nid,
            ctx.labels,
            ctx.node_community,
            ctx.deg_map,
            ctx.resolver,
        );
        std::fs::write(
            ctx.output_dir.join(format!("{slug}.md")),
            article.as_bytes(),
        )?;
    }
    Ok(god_articles.len())
}
