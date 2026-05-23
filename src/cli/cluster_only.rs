//! `cluster-only` command — rerun clustering on an existing graph.json and
//! regenerate the community report.

use anyhow::{Result, anyhow};

use crate::cli::{build_analysis, load_graph};

/// Rerun community detection on an existing graph.json and regenerate the report.
///
/// `exclude_hubs` is forwarded directly to `graphify_cluster::cluster` as the
/// `exclude_hubs_percentile` parameter, which excludes hub nodes above the given
/// degree percentile before partitioning (0.0–1.0 range maps to percentile).
/// `min_community_size` is honoured by filtering communities from the analysis
/// JSON that the report renderer reads; `graphify_cluster::cluster` itself does
/// not accept a minimum-size parameter.
#[allow(clippy::too_many_lines)] // CLI entry point: linear orchestration is clearer than splitting.
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
    // Anything outside [0.0, 1.0] is rejected so a stray `--exclude-hubs 95`
    // doesn't silently become an absurd 9500% percentile.
    let hubs_pct = match exclude_hubs {
        Some(p) if (0.0..=1.0).contains(&p) => Some(p * 100.0),
        Some(p) => {
            return Err(anyhow!(
                "--exclude-hubs must be a fraction in [0.0, 1.0]; got {p}"
            ));
        }
        None => None,
    };
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

    // Persist the analysis sidecar for downstream exports (wiki, obsidian, etc.).
    // Mirrors Python's `cluster-only` path which rewrites `.graphify_analysis.json`.
    let analysis_path = graph_path.with_file_name(".graphify_analysis.json");
    std::fs::write(&analysis_path, serde_json::to_string_pretty(&analysis)?)?;
    eprintln!("      wrote {}", analysis_path.display());

    // Refresh graph.json so node community attrs match the new partition.
    // Mirrors Python `__main__.py:1831` (`to_json(G, communities, ...)`).
    graphify_export::to_json(&g, &communities, &graph_path, true, None)?;
    eprintln!("      wrote {}", graph_path.display());

    // Persist (or refresh) `.graphify_labels.json` so the HTML viz and
    // subsequent exports can find community labels.  Loads existing labels
    // first to preserve user-edited names; falls back to `"Community <cid>"`.
    let labels_path = graph_path.with_file_name(".graphify_labels.json");
    let mut labels: indexmap::IndexMap<i64, String> = indexmap::IndexMap::new();
    if let Ok(text) = std::fs::read_to_string(&labels_path)
        && let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text)
    {
        for (k, v) in &map {
            if let (Ok(cid), Some(s)) = (k.parse::<i64>(), v.as_str()) {
                labels.insert(cid, s.to_string());
            }
        }
    }
    for cid in communities.keys() {
        labels
            .entry(*cid)
            .or_insert_with(|| format!("Community {cid}"));
    }
    let labels_json: serde_json::Map<String, serde_json::Value> = labels
        .iter()
        .map(|(cid, name)| (cid.to_string(), serde_json::Value::String(name.clone())))
        .collect();
    std::fs::write(
        &labels_path,
        serde_json::to_string(&serde_json::Value::Object(labels_json))?,
    )?;
    eprintln!("      wrote {}", labels_path.display());

    let html_path = graph_path.with_file_name("graph.html");
    if no_viz {
        if html_path.exists() {
            std::fs::remove_file(&html_path)?;
        }
        eprintln!("[4/4] HTML viz: skipped (--no-viz; graph.html removed)");
    } else {
        eprintln!("[4/4] rendering HTML viz ...");
        let labels_opt = if labels.is_empty() {
            None
        } else {
            Some(&labels)
        };
        match graphify_export::to_html(&g, &communities, &html_path, labels_opt, None, None) {
            Ok(()) => eprintln!("      wrote {}", html_path.display()),
            Err(e) => {
                if html_path.exists() {
                    let _ = std::fs::remove_file(&html_path);
                }
                eprintln!("      skipped ({e})");
            }
        }
    }
    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    Ok(())
}
