//! `export` subcommands — export graph to various formats (`HTML`, `SVG`, `GraphML`,
//! `Obsidian`, `Wiki`, Mermaid call-flow HTML, `Neo4j`).

use anyhow::Result;

use crate::ExportCmd;
use crate::cli::{default_graph_path, graphify_out_dir, load_graph};

#[allow(clippy::too_many_lines)]
// reason: dispatch over every ExportCmd variant; splitting adds indirection
// without reducing complexity; mirrors Python's elif chain in __main__.py.
/// Dispatch the `export` subcommand to the appropriate format renderer.
///
/// Each arm loads `graph.json`, clusters if needed, and writes the output
/// file. Mirrors the `export` elif chain in `__main__.py`.
pub(crate) fn cmd_export(cmd: ExportCmd) -> Result<()> {
    match cmd {
        ExportCmd::Graphml { graph } => {
            let path = graph.unwrap_or_else(default_graph_path);
            let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let analysis = load_analysis_sidecar(&out_dir.join(".graphify_analysis.json"));
            let out = path.with_file_name("graph.graphml");
            eprintln!(
                "exporting GraphML ({} nodes, {} edges) ...",
                g.node_count(),
                g.edge_count()
            );
            graphify_export::to_graphml(&g, &analysis.communities, &out)?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Svg { graph, labels } => {
            let path = graph.unwrap_or_else(default_graph_path);
            let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let analysis = load_analysis_sidecar(&out_dir.join(".graphify_analysis.json"));
            let labels_path = labels.unwrap_or_else(|| out_dir.join(".graphify_labels.json"));
            let community_labels = load_community_labels(&labels_path);
            let labels_opt = if community_labels.is_empty() {
                None
            } else {
                Some(&community_labels)
            };
            let out = path.with_file_name("graph.svg");
            eprintln!(
                "computing spring layout + rendering SVG for {} nodes ...",
                g.node_count()
            );
            graphify_export::to_svg(&g, &analysis.communities, &out, labels_opt, (16, 12))?;
            eprintln!("wrote {}", out.display());
        }
        ExportCmd::Html {
            graph,
            labels,
            node_limit,
            no_viz,
        } => {
            let path = graph.unwrap_or_else(default_graph_path);
            let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let analysis = load_analysis_sidecar(&out_dir.join(".graphify_analysis.json"));
            let labels_path = labels.unwrap_or_else(|| out_dir.join(".graphify_labels.json"));
            let community_labels = load_community_labels(&labels_path);
            let labels_opt = if community_labels.is_empty() {
                None
            } else {
                Some(&community_labels)
            };
            let out = path.with_file_name("graph.html");
            if no_viz {
                if out.exists() {
                    std::fs::remove_file(&out)?;
                }
                println!("--no-viz: skipped graph.html");
            } else {
                eprintln!("rendering HTML viz ({} nodes) ...", g.node_count());
                graphify_export::to_html(
                    &g,
                    &analysis.communities,
                    &out,
                    labels_opt,
                    None,
                    Some(node_limit),
                )?;
                if g.node_count() <= node_limit {
                    println!("graph.html written - open in any browser, no server needed");
                }
            }
        }
        ExportCmd::Obsidian { graph, out, labels } => {
            let path = graph.unwrap_or_else(default_graph_path);
            let parent = path.parent().unwrap_or(std::path::Path::new("."));
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let analysis = load_analysis_sidecar(&parent.join(".graphify_analysis.json"));
            let labels_path = labels.unwrap_or_else(|| parent.join(".graphify_labels.json"));
            let community_labels = load_community_labels(&labels_path);
            let out_dir = out.unwrap_or_else(|| graphify_out_dir().join("obsidian"));
            eprintln!(
                "rendering Obsidian vault ({} nodes) to {} ...",
                g.node_count(),
                out_dir.display()
            );
            let labels_opt = if community_labels.is_empty() {
                None
            } else {
                Some(&community_labels)
            };
            let cohesion_opt = if analysis.cohesion.is_empty() {
                None
            } else {
                Some(&analysis.cohesion)
            };
            let count = graphify_export::to_obsidian(
                &g,
                &analysis.communities,
                &out_dir,
                labels_opt,
                cohesion_opt,
            )?;
            println!("Obsidian vault: {count} notes in {}/", out_dir.display());
            // Mirror Python: also write graph.canvas next to the vault.
            let canvas_path = out_dir.join("graph.canvas");
            graphify_export::to_canvas(&g, &analysis.communities, &canvas_path, labels_opt, None)?;
            println!("Canvas: {}", canvas_path.display());
            println!("Open {}/ as a vault in Obsidian.", out_dir.display());
        }
        ExportCmd::Wiki { graph, labels } => {
            // Mirror Python's export wiki path at __main__.py:2283: load
            // communities from `.graphify_analysis.json`; bail if absent to
            // prevent silent divergence from extract-time clustering.
            let path = graph.unwrap_or_else(default_graph_path);
            let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;

            let analysis_path = out_dir.join(".graphify_analysis.json");
            let labels_path = labels.unwrap_or_else(|| out_dir.join(".graphify_labels.json"));

            let analysis = load_analysis_sidecar(&analysis_path);
            let communities = analysis.communities;
            if communities.is_empty() {
                anyhow::bail!(
                    ".graphify_analysis.json is missing or empty — refusing to export wiki to \
                     prevent data loss.\nRun `graphify extract .` (or `graphify cluster-only .`) \
                     to regenerate community data first."
                );
            }
            let community_labels = load_community_labels(&labels_path);
            let gods: Vec<graphify_wiki::GodNodeData> = if analysis.gods.is_empty() {
                graphify_analyze::god_nodes(&g, 10)
                    .into_iter()
                    .filter_map(|v| {
                        let o = v.as_object()?;
                        Some(graphify_wiki::GodNodeData {
                            id: o.get("id")?.as_str()?.to_string(),
                            label: o.get("label")?.as_str()?.to_string(),
                            degree: usize::try_from(
                                o.get("degree")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0),
                            )
                            .unwrap_or(0),
                        })
                    })
                    .collect()
            } else {
                analysis.gods
            };
            let wiki_dir = out_dir.join("wiki");
            eprintln!(
                "writing wiki ({} communities) to {} ...",
                communities.len(),
                wiki_dir.display()
            );
            let labels_opt = if community_labels.is_empty() {
                None
            } else {
                Some(&community_labels)
            };
            let cohesion_opt = if analysis.cohesion.is_empty() {
                None
            } else {
                Some(&analysis.cohesion)
            };
            let gods_opt = if gods.is_empty() {
                None
            } else {
                Some(gods.as_slice())
            };
            let n = graphify_wiki::to_wiki(
                &g,
                &communities,
                &wiki_dir,
                labels_opt,
                cohesion_opt,
                gods_opt,
            )?;
            println!("Wiki: {n} articles written to {}", wiki_dir.display());
            println!("  {}/index.md  ->  agent entry point", wiki_dir.display());
        }
        ExportCmd::CallflowHtml {
            graph,
            output,
            lang,
            max_sections,
            diagram_scale,
            max_diagram_nodes,
            max_diagram_edges,
            report,
            sections,
        } => {
            eprintln!("rendering Mermaid call-flow HTML ...");
            let mut opts = graphify_html::callflow::CallflowOptions {
                graph,
                output,
                report,
                sections,
                ..Default::default()
            };
            // Apply flag overrides only when the user explicitly provided them.
            if let Some(l) = lang {
                opts.lang = l;
            }
            if let Some(ms) = max_sections {
                opts.max_sections = ms;
            }
            if let Some(ds) = diagram_scale {
                opts.diagram_scale = ds;
            }
            if let Some(mdn) = max_diagram_nodes {
                opts.max_diagram_nodes = mdn;
            }
            if let Some(mde) = max_diagram_edges {
                opts.max_diagram_edges = mde;
            }
            let written = graphify_html::callflow::write_callflow_html(&opts)?;
            // Match Python's stdout message at __main__.py:2229 so callers
            // grepping stdout for "callflow HTML written" find it.
            println!(
                "callflow HTML written - open in any browser: {}",
                written.display()
            );
        }
        ExportCmd::Neo4j {
            graph,
            push,
            user,
            password,
        } => {
            let path = graph.unwrap_or_else(default_graph_path);
            eprintln!("loading {} ...", path.display());
            let g = load_graph(&path)?;
            let out_dir = path.parent().unwrap_or(std::path::Path::new("."));

            if let Some(uri) = push {
                let analysis = load_analysis_sidecar(&out_dir.join(".graphify_analysis.json"));
                let resolved_password = password
                    .or_else(|| std::env::var("NEO4J_PASSWORD").ok())
                    .ok_or_else(|| {
                    anyhow::anyhow!(
                        "--push requires a password (--password or NEO4J_PASSWORD env var)"
                    )
                })?;
                eprintln!(
                    "pushing {} nodes / {} edges to {uri} ...",
                    g.node_count(),
                    g.edge_count()
                );
                let (n_nodes, n_rels) = graphify_export::push_to_neo4j_blocking(
                    &uri,
                    &user,
                    &resolved_password,
                    &g,
                    &analysis.communities,
                    false,
                )?;
                println!("pushed {n_nodes} nodes, {n_rels} relationships to {uri}");
            } else {
                let out = out_dir.join("cypher.txt");
                graphify_export::to_cypher(&g, &out)?;
                println!(
                    "cypher.txt written - import with: cypher-shell < {}",
                    out.display()
                );
            }
        }
    }
    Ok(())
}

/// Parsed contents of `.graphify_analysis.json` consumed by exports.
struct AnalysisSidecar {
    communities: indexmap::IndexMap<i64, Vec<String>>,
    cohesion: indexmap::IndexMap<i64, f64>,
    gods: Vec<graphify_wiki::GodNodeData>,
}

/// Load `.graphify_analysis.json` and extract the fields exports consume.
///
/// Missing or malformed files produce empty maps/vectors instead of errors,
/// letting callers decide whether to bail (e.g. wiki refuses an empty
/// community map to avoid silent divergence).
fn load_analysis_sidecar(path: &std::path::Path) -> AnalysisSidecar {
    let mut sidecar = AnalysisSidecar {
        communities: indexmap::IndexMap::new(),
        cohesion: indexmap::IndexMap::new(),
        gods: Vec::new(),
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return sidecar;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return sidecar;
    };
    if let Some(map) = json
        .get("communities")
        .and_then(serde_json::Value::as_object)
    {
        for (key, val) in map {
            let Ok(cid) = key.parse::<i64>() else {
                continue;
            };
            let Some(arr) = val.as_array() else { continue };
            let members: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            sidecar.communities.insert(cid, members);
        }
    }
    // Python writes "cohesion"; the Rust pipeline writes "cohesion_scores".
    // Accept either to keep cross-version sidecars consumable.
    let cohesion_obj = json
        .get("cohesion_scores")
        .or_else(|| json.get("cohesion"))
        .and_then(serde_json::Value::as_object);
    if let Some(map) = cohesion_obj {
        for (key, val) in map {
            let Ok(cid) = key.parse::<i64>() else {
                continue;
            };
            if let Some(f) = val.as_f64() {
                sidecar.cohesion.insert(cid, f);
            }
        }
    }
    // Python writes "gods"; Rust pipeline writes "god_nodes". Accept both.
    let gods_arr = json
        .get("god_nodes")
        .or_else(|| json.get("gods"))
        .and_then(serde_json::Value::as_array);
    if let Some(arr) = gods_arr {
        for entry in arr {
            let Some(obj) = entry.as_object() else {
                continue;
            };
            let Some(id) = obj.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(label) = obj.get("label").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let degree = usize::try_from(
                obj.get("degree")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            )
            .unwrap_or(0);
            sidecar.gods.push(graphify_wiki::GodNodeData {
                id: id.to_string(),
                label: label.to_string(),
                degree,
            });
        }
    }
    sidecar
}

/// Load `.graphify_labels.json` — a `{"<cid>": "<label>"}` map.
///
/// Missing or malformed files yield an empty map; callers decide whether
/// to pass `None` to the renderer.
fn load_community_labels(path: &std::path::Path) -> indexmap::IndexMap<i64, String> {
    let mut out: indexmap::IndexMap<i64, String> = indexmap::IndexMap::new();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return out;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return out;
    };
    let Some(map) = json.as_object() else {
        return out;
    };
    for (key, val) in map {
        let Ok(cid) = key.parse::<i64>() else {
            continue;
        };
        if let Some(s) = val.as_str() {
            out.insert(cid, s.to_string());
        }
    }
    out
}
