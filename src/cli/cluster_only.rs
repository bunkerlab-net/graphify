//! `cluster-only` command — rerun clustering on an existing graph.json and
//! regenerate the community report.

use anyhow::Result;

use crate::cli::{build_analysis, load_graph};

/// Rerun community detection on an existing graph.json and regenerate the report.
///
/// `exclude_hubs` is forwarded directly to `graphify_cluster::cluster` as the
/// `exclude_hubs_percentile` parameter, which excludes hub nodes above the given
/// degree percentile before partitioning (0.0–1.0 range maps to percentile).
/// `min_community_size` is honoured by filtering communities from the analysis
/// JSON that the report renderer reads; `graphify_cluster::cluster` itself does
/// not accept a minimum-size parameter.
pub(crate) fn cmd_cluster_only(
    path: &std::path::Path,
    no_viz: bool,
    graph: Option<&std::path::Path>,
    resolution: f64,
    exclude_hubs: Option<f64>,
    min_community_size: usize,
) -> Result<()> {
    let start = std::time::Instant::now();
    let graph_path = graph.map_or_else(
        || path.join(crate::cli::graphify_out_dir()).join("graph.json"),
        std::path::Path::to_path_buf,
    );
    eprintln!("[1/4] loading {} ...", graph_path.display());
    let g = load_graph(&graph_path)?;
    eprintln!(
        "      loaded {} nodes, {} edges",
        g.node_count(),
        g.edge_count()
    );

    let hub_desc = exclude_hubs
        .map(|p| format!(", exclude-hubs={p}"))
        .unwrap_or_default();
    eprintln!("[2/4] clustering (Louvain, resolution={resolution}{hub_desc}) ...");
    let cluster_start = std::time::Instant::now();
    // Forward exclude_hubs directly; convert 0.0–1.0 fraction to 0.0–100.0 percentile
    // as expected by graphify_cluster (mirroring Python's `--exclude-hubs` semantics).
    let hubs_pct = exclude_hubs.map(|p| p * 100.0);
    let communities = graphify_cluster::cluster(&g, resolution, hubs_pct);
    eprintln!(
        "      found {} communities in {:.1}s",
        communities.len(),
        cluster_start.elapsed().as_secs_f64()
    );

    // Apply min_community_size filter: drop communities below the threshold from
    // the analysis (the full map is still passed to the HTML renderer so the viz
    // is unchanged, mirroring Python's report-only filtering at __main__.py:1820).
    let report_communities: indexmap::IndexMap<i64, Vec<String>> = if min_community_size > 1 {
        let filtered: indexmap::IndexMap<i64, Vec<String>> = communities
            .iter()
            .filter(|(_, members)| members.len() >= min_community_size)
            .map(|(&cid, members)| (cid, members.clone()))
            .collect();
        eprintln!(
            "      after min-community-size={min_community_size}: {} communities",
            filtered.len()
        );
        filtered
    } else {
        communities.clone()
    };

    eprintln!("[3/4] writing report ...");
    let analysis = build_analysis(&g, &report_communities, path);
    let report_path = graph_path.with_file_name("GRAPH_REPORT.md");
    graphify_report::write_report(&g, &analysis, &report_path)?;
    eprintln!("      wrote {}", report_path.display());

    if no_viz {
        eprintln!("[4/4] HTML viz: skipped (--no-viz)");
    } else {
        eprintln!("[4/4] rendering HTML viz ...");
        let html_path = graph_path.with_file_name("graph.html");
        match graphify_export::to_html(&g, &communities, &html_path, None, None, None) {
            Ok(()) => eprintln!("      wrote {}", html_path.display()),
            Err(e) => eprintln!("      skipped ({e})"),
        }
    }
    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    Ok(())
}
