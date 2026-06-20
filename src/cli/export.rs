//! `export` subcommands — export graph to various formats (`HTML`, `SVG`, `GraphML`,
//! `Obsidian`, `Wiki`, Mermaid call-flow HTML, `Neo4j`).

use anyhow::Result;

use crate::cli::args::ExportCmd;
use crate::cli::{default_graph_path, graphify_out_dir, load_graph};

/// Dispatch the `export` subcommand to the appropriate format renderer.
///
/// Each arm loads `graph.json`, clusters if needed, and writes the output
/// file. Mirrors the `export` elif chain in `__main__.py`.
pub(crate) fn cmd_export(cmd: ExportCmd) -> Result<()> {
    match cmd {
        ExportCmd::Graphml { graph } => export_graphml(graph),
        ExportCmd::Svg { graph, labels } => export_svg(graph, labels),
        ExportCmd::Html {
            graph,
            labels,
            node_limit,
            no_viz,
        } => export_html(graph, labels, node_limit, no_viz),
        ExportCmd::Obsidian { graph, out, labels } => export_obsidian(graph, out, labels),
        ExportCmd::Wiki { graph, labels } => export_wiki(graph, labels),
        ExportCmd::CallflowHtml {
            path,
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
            // `--graph` wins; otherwise the optional positional GRAPH|DIR is
            // resolved to a graph.json path (mirrors __main__.py callflow-html).
            let graph = graph.or_else(|| path.map(resolve_callflow_graph_arg));
            let opts = CallflowDispatchOptions {
                graph,
                output,
                lang,
                max_sections,
                diagram_scale,
                max_diagram_nodes,
                max_diagram_edges,
                report,
                sections,
            };
            export_callflow_html(opts)
        }
        ExportCmd::Neo4j {
            graph,
            push,
            user,
            password,
        } => export_neo4j(graph, push, &user, password),
        ExportCmd::Falkordb {
            graph,
            push,
            user,
            password,
        } => export_falkordb(graph, push, user.as_deref(), password),
    }
}

/// Resolve a positional `export callflow-html [GRAPH|DIR]` argument to a
/// `graph.json` path.
///
/// Mirrors `__main__.py`: a `graph.json` / `*.json` path is used as-is; a
/// directory containing `graph.json` resolves to it; otherwise the argument is
/// treated as a project root and resolved to `<dir>/<GRAPHIFY_OUT>/graph.json`.
fn resolve_callflow_graph_arg(candidate: std::path::PathBuf) -> std::path::PathBuf {
    let is_json = candidate.file_name().and_then(|n| n.to_str()) == Some("graph.json")
        || candidate
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if is_json {
        candidate
    } else if candidate.join("graph.json").exists() {
        candidate.join("graph.json")
    } else {
        candidate.join(graphify_out_dir()).join("graph.json")
    }
}

fn export_graphml(graph: Option<std::path::PathBuf>) -> Result<()> {
    let path = graph.unwrap_or_else(default_graph_path);
    let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    let analysis =
        load_analysis_with_community_fallback(&out_dir.join(".graphify_analysis.json"), &g);
    let out = path.with_file_name("graph.graphml");
    eprintln!(
        "exporting GraphML ({} nodes, {} edges) ...",
        g.node_count(),
        g.edge_count()
    );
    graphify_export::to_graphml(&g, &analysis.communities, &out)?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

fn export_svg(graph: Option<std::path::PathBuf>, labels: Option<std::path::PathBuf>) -> Result<()> {
    let path = graph.unwrap_or_else(default_graph_path);
    let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    let analysis =
        load_analysis_with_community_fallback(&out_dir.join(".graphify_analysis.json"), &g);
    let labels_path = labels.unwrap_or_else(|| out_dir.join(".graphify_labels.json"));
    let community_labels = load_community_labels(&labels_path);
    let labels_opt = (!community_labels.is_empty()).then_some(&community_labels);
    let out = path.with_file_name("graph.svg");
    eprintln!(
        "computing spring layout + rendering SVG for {} nodes ...",
        g.node_count()
    );
    graphify_export::to_svg(&g, &analysis.communities, &out, labels_opt, (16, 12))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

fn export_html(
    graph: Option<std::path::PathBuf>,
    labels: Option<std::path::PathBuf>,
    node_limit: usize,
    no_viz: bool,
) -> Result<()> {
    let path = graph.unwrap_or_else(default_graph_path);
    let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    let analysis =
        load_analysis_with_community_fallback(&out_dir.join(".graphify_analysis.json"), &g);
    let labels_path = labels.unwrap_or_else(|| out_dir.join(".graphify_labels.json"));
    let community_labels = load_community_labels(&labels_path);
    let labels_opt = (!community_labels.is_empty()).then_some(&community_labels);
    let out = path.with_file_name("graph.html");
    if no_viz {
        if out.exists() {
            std::fs::remove_file(&out)?;
        }
        println!("--no-viz: skipped graph.html");
        return Ok(());
    }
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
    Ok(())
}

fn export_obsidian(
    graph: Option<std::path::PathBuf>,
    out: Option<std::path::PathBuf>,
    labels: Option<std::path::PathBuf>,
) -> Result<()> {
    let path = graph.unwrap_or_else(default_graph_path);
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    let analysis =
        load_analysis_with_community_fallback(&parent.join(".graphify_analysis.json"), &g);
    let labels_path = labels.unwrap_or_else(|| parent.join(".graphify_labels.json"));
    let community_labels = load_community_labels(&labels_path);
    let out_dir = out.unwrap_or_else(|| graphify_out_dir().join("obsidian"));
    eprintln!(
        "rendering Obsidian vault ({} nodes) to {} ...",
        g.node_count(),
        out_dir.display()
    );
    let labels_opt = (!community_labels.is_empty()).then_some(&community_labels);
    let cohesion_opt = (!analysis.cohesion.is_empty()).then_some(&analysis.cohesion);
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
    Ok(())
}

fn export_wiki(
    graph: Option<std::path::PathBuf>,
    labels: Option<std::path::PathBuf>,
) -> Result<()> {
    // Mirror Python's export wiki path at __main__.py:2283: load communities from
    // `.graphify_analysis.json`; bail if absent to prevent silent divergence.
    let path = graph.unwrap_or_else(default_graph_path);
    let out_dir = path.parent().unwrap_or(std::path::Path::new("."));
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;

    let analysis_path = out_dir.join(".graphify_analysis.json");
    let labels_path = labels.unwrap_or_else(|| out_dir.join(".graphify_labels.json"));

    // The fallback applies here too: the wiki bail-out exists to prevent
    // silent divergence when no community data is available, but per-node
    // `community` attributes count as valid data — only bail when both the
    // sidecar AND the reconstructed map are empty.
    let analysis = load_analysis_with_community_fallback(&analysis_path, &g);
    let communities = analysis.communities;
    if communities.is_empty() {
        anyhow::bail!(
            ".graphify_analysis.json is missing or empty and graph.json has no \
             per-node community attribute — refusing to export wiki to prevent \
             data loss.\nRun `graphify extract .` (or `graphify cluster-only .`) \
             to regenerate community data first."
        );
    }
    let community_labels = load_community_labels(&labels_path);
    let gods = resolve_wiki_gods(&g, analysis.gods);
    let wiki_dir = out_dir.join("wiki");
    eprintln!(
        "writing wiki ({} communities) to {} ...",
        communities.len(),
        wiki_dir.display()
    );
    let labels_opt = (!community_labels.is_empty()).then_some(&community_labels);
    let cohesion_opt = (!analysis.cohesion.is_empty()).then_some(&analysis.cohesion);
    let gods_opt = (!gods.is_empty()).then_some(gods.as_slice());
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
    Ok(())
}

/// Recompute god nodes when the analysis sidecar didn't carry them.
fn resolve_wiki_gods(
    g: &graphify_build::Graph,
    sidecar: Vec<graphify_wiki::GodNodeData>,
) -> Vec<graphify_wiki::GodNodeData> {
    if !sidecar.is_empty() {
        return sidecar;
    }
    graphify_analyze::god_nodes(g, 10)
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
}

/// Bundled arguments extracted from `ExportCmd::CallflowHtml` for dispatch.
struct CallflowDispatchOptions {
    graph: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    lang: Option<String>,
    max_sections: Option<usize>,
    diagram_scale: Option<f64>,
    max_diagram_nodes: Option<usize>,
    max_diagram_edges: Option<usize>,
    report: Option<std::path::PathBuf>,
    sections: Option<std::path::PathBuf>,
}

fn export_callflow_html(opts: CallflowDispatchOptions) -> Result<()> {
    eprintln!("rendering Mermaid call-flow HTML ...");
    let mut callflow_opts = graphify_html::callflow::CallflowOptions {
        graph: opts.graph,
        output: opts.output,
        report: opts.report,
        sections: opts.sections,
        ..Default::default()
    };
    // Apply flag overrides only when the user explicitly provided them.
    if let Some(l) = opts.lang {
        callflow_opts.lang = l;
    }
    if let Some(ms) = opts.max_sections {
        callflow_opts.max_sections = ms;
    }
    if let Some(ds) = opts.diagram_scale {
        callflow_opts.diagram_scale = ds;
    }
    if let Some(mdn) = opts.max_diagram_nodes {
        callflow_opts.max_diagram_nodes = mdn;
    }
    if let Some(mde) = opts.max_diagram_edges {
        callflow_opts.max_diagram_edges = mde;
    }
    let written = graphify_html::callflow::write_callflow_html(&callflow_opts)?;
    // Match Python's stdout message at __main__.py:2229 so callers grepping stdout
    // for "callflow HTML written" find it.
    println!(
        "callflow HTML written - open in any browser: {}",
        written.display()
    );
    Ok(())
}

fn export_neo4j(
    graph: Option<std::path::PathBuf>,
    push: Option<String>,
    user: &str,
    password: Option<String>,
) -> Result<()> {
    let path = graph.unwrap_or_else(default_graph_path);
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    let out_dir = path.parent().unwrap_or(std::path::Path::new("."));

    let Some(uri) = push else {
        let out = out_dir.join("cypher.txt");
        graphify_export::to_cypher(&g, &out)?;
        println!(
            "cypher.txt written - import with: cypher-shell < {}",
            out.display()
        );
        return Ok(());
    };

    let analysis =
        load_analysis_with_community_fallback(&out_dir.join(".graphify_analysis.json"), &g);
    let resolved_password = password
        .or_else(|| std::env::var("NEO4J_PASSWORD").ok())
        .ok_or_else(|| {
            anyhow::anyhow!("--push requires a password (--password or NEO4J_PASSWORD env var)")
        })?;
    eprintln!(
        "pushing {} nodes / {} edges to {uri} ...",
        g.node_count(),
        g.edge_count()
    );
    let (n_nodes, n_rels) = graphify_export::push_to_neo4j_blocking(
        &uri,
        user,
        &resolved_password,
        &g,
        &analysis.communities,
        false,
    )?;
    println!("pushed {n_nodes} nodes, {n_rels} relationships to {uri}");
    Ok(())
}

/// `export falkordb` — write `cypher.txt` (`OpenCypher`, no `--push`) or push the
/// graph directly to a live `FalkorDB` instance via the `GRAPH.QUERY` command.
///
/// The live push needs the `falkordb` cargo feature (which pulls the `redis`
/// client); a build without it falls back to a clear error on `--push` while
/// the `cypher.txt` path still works. Mirrors Python's `export falkordb`.
fn export_falkordb(
    graph: Option<std::path::PathBuf>,
    push: Option<String>,
    user: Option<&str>,
    password: Option<String>,
) -> Result<()> {
    let path = graph.unwrap_or_else(default_graph_path);
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    let out_dir = path.parent().unwrap_or(std::path::Path::new("."));

    let Some(uri) = push else {
        let out = out_dir.join("cypher.txt");
        graphify_export::to_cypher(&g, &out)?;
        println!(
            "cypher.txt written - FalkorDB is OpenCypher-compatible; import with: \
             redis-cli -x GRAPH.QUERY graphify < {}",
            out.display()
        );
        return Ok(());
    };

    #[cfg(feature = "falkordb")]
    {
        let analysis =
            load_analysis_with_community_fallback(&out_dir.join(".graphify_analysis.json"), &g);
        let resolved_password = password.or_else(|| std::env::var("FALKORDB_PASSWORD").ok());
        eprintln!(
            "pushing {} nodes / {} edges to {uri} ...",
            g.node_count(),
            g.edge_count()
        );
        let (n_nodes, n_edges) = graphify_export::falkordb::push_to_falkordb(
            &uri,
            user,
            resolved_password.as_deref(),
            &g,
            &analysis.communities,
            "graphify",
        )?;
        println!("Pushed to FalkorDB: {n_nodes} nodes, {n_edges} edges");
        Ok(())
    }
    #[cfg(not(feature = "falkordb"))]
    {
        // Consume the (otherwise unused) owned values on the no-feature build so
        // they aren't flagged as needlessly passed by value.
        drop((uri, password));
        let _ = user;
        anyhow::bail!(
            "FalkorDB live push requires a build with the `falkordb` feature \
             (`cargo build --features falkordb`). Omit --push to write cypher.txt instead."
        )
    }
}

/// Parsed contents of `.graphify_analysis.json` consumed by exports.
struct AnalysisSidecar {
    communities: indexmap::IndexMap<i64, Vec<String>>,
    cohesion: indexmap::IndexMap<i64, f64>,
    gods: Vec<graphify_wiki::GodNodeData>,
}

/// Fall back to reconstructing the `cid → [node_ids]` map from the per-node
/// `community` attribute on `graph.json` when the analysis sidecar is missing
/// or empty.
///
/// The watch / post-commit rebuild path and some skill workflows only
/// regenerate `graph.json` + `GRAPH_REPORT.md`, leaving
/// `.graphify_analysis.json` stale or absent. The per-node attribute (written
/// by `to_json` on every node) is still authoritative, so downstream
/// subcommands (`html`, `obsidian`, `wiki`, `svg`, `graphml`, `neo4j`) can
/// reconstruct the same shape they would have read from the sidecar.
///
/// Mirrors Python `__main__.py` (graphify-py @ d778e2c).
#[must_use]
fn reconstruct_communities_from_graph(
    graph: &graphify_build::Graph,
) -> indexmap::IndexMap<i64, Vec<String>> {
    let mut out: indexmap::IndexMap<i64, Vec<String>> = indexmap::IndexMap::new();
    for (node_id, attrs) in graph.nodes() {
        let Some(raw) = attrs.get("community") else {
            continue;
        };
        let cid = match raw {
            serde_json::Value::Number(n) => n.as_i64(),
            serde_json::Value::String(s) => s.parse::<i64>().ok(),
            _ => None,
        };
        if let Some(cid) = cid {
            out.entry(cid).or_default().push(node_id.clone());
        }
    }
    out
}

/// Load the analysis sidecar and, if its `communities` map is empty, fill it
/// from the per-node `community` attribute on `graph`. This is the join point
/// for the d778e2c fallback — every export command goes through here so the
/// behaviour is uniform.
#[must_use]
fn load_analysis_with_community_fallback(
    sidecar_path: &std::path::Path,
    graph: &graphify_build::Graph,
) -> AnalysisSidecar {
    let mut analysis = load_analysis_sidecar(sidecar_path);
    if analysis.communities.is_empty() {
        let reconstructed = reconstruct_communities_from_graph(graph);
        if !reconstructed.is_empty() {
            analysis.communities = reconstructed;
        }
    }
    analysis
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
