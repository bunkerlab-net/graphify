//! `cluster-only` command — rerun clustering on an existing graph.json and
//! regenerate the community report.

use anyhow::{Result, anyhow};

use crate::cli::{build_analysis, load_graph};

/// Community-labelling knobs for [`cmd_cluster_only`].
#[derive(Clone, Copy, Default)]
// Each field is an independent CLI flag (one `--flag` apiece); grouping them
// into enums would be artificial — this is the options-bag the lint exempts.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct LabelOptions<'a> {
    /// Keep `Community N` placeholders instead of LLM-naming (the `--no-label` flag).
    pub no_label: bool,
    /// Backend override for naming; `None` auto-detects from API keys.
    pub backend: Option<&'a str>,
    /// Model override for naming; `None` uses the backend default (`--model`).
    pub model: Option<&'a str>,
    /// `graphify label` always (re)names even when a labels file exists.
    pub force_relabel: bool,
    /// Max community-label batches sent concurrently (#1390).
    pub max_concurrency: usize,
    /// Communities per LLM labeling call (#1390).
    pub batch_size: usize,
    /// Print per-stage wall-clock timings to stderr (#1490).
    pub timing: bool,
    /// Only (re)name communities that are unnamed or hold a `Community N`
    /// placeholder, preserving existing labels (#1481).
    pub missing_only: bool,
}

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
    opts: LabelOptions<'_>,
) -> Result<()> {
    let start = std::time::Instant::now();
    let mut stages = super::timer::StageTimer::new(opts.timing);
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
    stages.mark("load");

    let hub_desc = exclude_hubs
        .map(|p| format!(", exclude-hubs={p}"))
        .unwrap_or_default();
    let backend = std::env::var("GRAPHIFY_CLUSTER_BACKEND")
        .ok()
        .filter(|s| s.eq_ignore_ascii_case("louvain"))
        .map_or("Leiden", |_| "Louvain");
    eprintln!("[2/4] clustering ({backend}, resolution={resolution}{hub_desc}) ...");
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
    stages.mark("cluster");

    // Mirror the watch/update path (#822, #1028): map new community IDs back to
    // the prior ones by node overlap so an existing .graphify_labels.json keeps
    // attaching to the same conceptual community after re-clustering. Without
    // this, labels follow the raw cid index and misalign whenever the graph
    // changed between labeling and cluster-only.
    let previous_node_community: indexmap::IndexMap<String, i64> = g
        .nodes()
        .filter_map(|(id, attrs)| {
            attrs
                .get("community")
                .and_then(serde_json::Value::as_i64)
                .map(|c| (id.clone(), c))
        })
        .collect();
    let communities = if previous_node_community.is_empty() {
        communities
    } else {
        graphify_cluster::remap_communities_to_previous(&communities, &previous_node_community)
    };

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
    stages.mark("analyze");

    // Resolve `.graphify_labels.json` so the HTML viz and downstream exports can
    // find community labels. Three paths, checked in this order:
    //   1. labels file exists & not forced & we are NOT LLM-naming gaps — i.e.
    //      not `--missing-only`, OR `--no-label` (which forbids any LLM call,
    //      so `--no-label --missing-only` lands here too) → load it (preserve
    //      user edits, fill any gaps with placeholders). Crucially this must
    //      NOT wipe hand-curated labels
    //      to placeholders. A malformed/unreadable file is NOT overwritten — we
    //      warn and fall back to placeholders for this run so the file isn't
    //      silently clobbered (divergence from Python `__main__.py:2418-2448`,
    //      which degrades to placeholders and rewrites the file).
    //   2. `--no-label` (and not forced) with no labels file → placeholders, no
    //      LLM call.
    //   3. otherwise → auto-name with the configured backend (#1097); degrades
    //      to placeholders on no-backend/error.
    let labels_path = graph_path.with_file_name(".graphify_labels.json");
    let mut skip_label_write = false;
    let labels: indexmap::IndexMap<i64, String> =
        if labels_path.exists() && !opts.force_relabel && (!opts.missing_only || opts.no_label) {
            match read_existing_labels(&labels_path) {
                Ok(mut existing) => {
                    for cid in communities.keys() {
                        existing
                            .entry(*cid)
                            .or_insert_with(|| format!("Community {cid}"));
                    }
                    existing
                }
                Err(e) => {
                    eprintln!(
                        "      warning: could not read {} ({e}); using placeholders and \
                     leaving the existing file untouched",
                        labels_path.display()
                    );
                    skip_label_write = true;
                    graphify_llm::placeholder_community_labels(&communities)
                }
            }
        } else if opts.no_label && !opts.force_relabel {
            graphify_llm::placeholder_community_labels(&communities)
        } else if opts.missing_only
            && labels_path.exists()
            && read_existing_labels(&labels_path).is_err()
        {
            // Malformed-but-present labels file under `--missing-only`: preserve it
            // (don't relabel + overwrite), matching the non-`--missing-only` path
            // above. Degrade to placeholders for this run; the file is left intact.
            eprintln!(
                "      warning: could not read {} for --missing-only; using \
                 placeholders and leaving the existing file untouched",
                labels_path.display()
            );
            skip_label_write = true;
            graphify_llm::placeholder_community_labels(&communities)
        } else {
            // LLM community naming (#1097). With `--missing-only` (#1481), load any
            // existing labels and name only the communities that are unnamed or hold
            // a `Community N` placeholder, preserving the rest.
            let existing: indexmap::IndexMap<i64, String> = if opts.missing_only {
                read_existing_labels(&labels_path).unwrap_or_default()
            } else {
                indexmap::IndexMap::new()
            };
            let to_label: indexmap::IndexMap<i64, Vec<String>> = if opts.missing_only {
                communities
                    .iter()
                    .filter(|(cid, _)| {
                        existing
                            .get(*cid)
                            .is_none_or(|name| is_placeholder_label(name))
                    })
                    .map(|(&cid, members)| (cid, members.clone()))
                    .collect()
            } else {
                communities.clone()
            };
            if to_label.is_empty() {
                eprintln!("      all communities already named (--missing-only)");
                existing
            } else {
                eprintln!("Labeling communities...");
                let node_labels = node_label_map(&g);
                let gods = god_node_ids(&g);
                let (mut labels, _source) = graphify_llm::generate_community_labels(
                    &to_label,
                    &node_labels,
                    &gods,
                    opts.backend,
                    opts.model,
                    false, // quiet
                    opts.max_concurrency,
                    opts.batch_size,
                );
                // Keep existing good labels for communities we skipped, then backfill
                // any still-missing community with a placeholder.
                for (cid, name) in existing {
                    labels.entry(cid).or_insert(name);
                }
                for cid in communities.keys() {
                    labels
                        .entry(*cid)
                        .or_insert_with(|| format!("Community {cid}"));
                }
                labels
            }
        };
    stages.mark("label");

    // Refresh graph.json so node community attrs match the new partition and
    // carry the human community_name labels resolved above. Mirrors Python
    // `__main__.py:3283` (`to_json(G, communities, ..., community_labels=labels)`).
    graphify_export::to_json(&g, &communities, &graph_path, true, None, Some(&labels))?;
    eprintln!("      wrote {}", graph_path.display());

    if skip_label_write {
        eprintln!(
            "      kept existing {} (not overwritten)",
            labels_path.display()
        );
    } else {
        let labels_json: serde_json::Map<String, serde_json::Value> = labels
            .iter()
            .map(|(cid, name)| (cid.to_string(), serde_json::Value::String(name.clone())))
            .collect();
        std::fs::write(
            &labels_path,
            serde_json::to_string(&serde_json::Value::Object(labels_json))?,
        )?;
        eprintln!("      wrote {}", labels_path.display());
    }
    stages.mark("export");

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
    stages.total();
    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    Ok(())
}

/// True when a community label is absent or still a `Community N` placeholder,
/// so `--missing-only` (#1481) should (re)name it.
#[must_use]
fn is_placeholder_label(name: &str) -> bool {
    name.strip_prefix("Community ")
        .map_or(name.is_empty(), |rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
        })
}

/// Read an existing `.graphify_labels.json` into a `cid → name` map.
///
/// Returns `Err` when the file is unreadable or is not a JSON object, so the
/// caller can avoid overwriting a malformed or hand-curated file with
/// placeholders.
fn read_existing_labels(path: &std::path::Path) -> Result<indexmap::IndexMap<i64, String>> {
    let text = std::fs::read_to_string(path).map_err(|e| anyhow!("read failed: {e}"))?;
    let serde_json::Value::Object(map) = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| anyhow!("parse failed: {e}"))?
    else {
        return Err(anyhow!("not a JSON object"));
    };
    let mut existing: indexmap::IndexMap<i64, String> = indexmap::IndexMap::new();
    for (k, v) in &map {
        if let (Ok(cid), Some(s)) = (k.parse::<i64>(), v.as_str()) {
            existing.insert(cid, s.to_string());
        }
    }
    Ok(existing)
}

/// Build `node_id → label` for community labelling prompts.
#[must_use]
fn node_label_map(g: &graphify_build::Graph) -> indexmap::IndexMap<String, String> {
    g.nodes()
        .filter_map(|(id, attrs)| {
            attrs
                .get("label")
                .and_then(serde_json::Value::as_str)
                .map(|label| (id.clone(), label.to_string()))
        })
        .collect()
}

/// Cap on the number of god-nodes sampled to bias community-label prompts,
/// matching the `top_n=20` Python uses at the labelling boundary.
const GOD_NODE_CAP: usize = 20;

/// The set of god-node ids (used to bias which member labels are sampled first).
#[must_use]
fn god_node_ids(g: &graphify_build::Graph) -> indexmap::IndexSet<String> {
    graphify_analyze::god_nodes(g, GOD_NODE_CAP)
        .into_iter()
        .filter_map(|n| {
            n.get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect()
}
