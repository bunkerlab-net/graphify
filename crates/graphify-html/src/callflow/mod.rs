//! Call-flow architecture HTML generator.
//!
//! Ports `graphify-py/graphify/callflow_html.py`.
//!
//! Produces a self-contained dark-themed HTML page with:
//! * Sticky navigation bar.
//! * Mermaid flowchart architecture overview (aggregated section-level edges).
//! * Per-section Mermaid flowcharts (representative intra-section edges).
//! * Call-detail tables (headers + representative node rows).
//! * Auto-generated section intros and key-file cards.
//!
//! ## Module layout
//!
//! | Sub-module   | Contents |
//! |--------------|----------|
//! | `options`    | `CallflowOptions`, `Node`, `CfEdge`, `Section` |
//! | `template`   | Static CSS and JS blobs |
//! | `loader`     | JSON loading, normalization, Mermaid-id helpers |
//! | `builder`    | Community/section indexing, edge classification |
//! | `diagram`    | Mermaid diagram string generators |
//! | `render`     | HTML fragment generators |

mod archetypes;
mod builder;
mod diagram;
mod loader;
mod options;
mod render;
mod render_node;
mod template;

// Re-export public surface.
pub use archetypes::derive_sections_from_communities;
pub use builder::normalize_sections;
pub use loader::{
    html_comment_text, infer_project_name, load_graph, load_labels, load_report,
    mermaid_section_id, node_mermaid_id, normalize_edge, normalize_node, safe_file_path,
    safe_filename, safe_mermaid_text, stable_ascii_id,
};
pub use options::{CallflowOptions, CfEdge, Node, Section};

use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use crate::HtmlError;

/// Generate a call-flow architecture HTML file from graphify output files.
///
/// # Errors
///
/// Returns [`HtmlError::Io`] if the graph file cannot be read or the output
/// file cannot be written.
/// Returns [`HtmlError::EmptyGraph`] if the graph contains zero nodes.
/// Returns [`HtmlError::NoSections`] if no sections could be derived.
#[allow(clippy::too_many_lines)] // This is a monolithic HTML assembly function; splitting it would hurt readability.
pub fn write_callflow_html(opts: &CallflowOptions) -> Result<PathBuf, HtmlError> {
    let paths = loader::resolve_graphify_paths(opts);

    if !paths.graph.exists() {
        return Err(HtmlError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "graphify output not found: {}. Run graphify first or pass --graph /path/to/graph.json.",
                paths.graph.display()
            ),
        )));
    }

    let (nodes, edges, hyperedges, mut meta) = loader::load_graph(&paths.graph)?;
    let labels = loader::load_labels(Some(&paths.labels));
    let lang = loader::detect_lang(&opts.lang, &nodes, &labels);

    let sections: Vec<Section> = if let Some(ref sp) = paths.sections {
        // Load sections from JSON.
        let text = std::fs::read_to_string(sp)?;
        let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            HtmlError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            ))
        })?;
        let arr = match &data {
            serde_json::Value::Array(a) => a.as_slice(),
            serde_json::Value::Object(m) => m
                .get("sections")
                .and_then(|v| v.as_array())
                .map(std::vec::Vec::as_slice)
                .unwrap_or_default(),
            _ => &[],
        };
        arr.iter()
            .filter_map(|v| v.as_object())
            .map(|m| {
                let id = m
                    .get("id")
                    .or_else(|| m.get("key"))
                    .or_else(|| m.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let name = m
                    .get("name")
                    .or_else(|| m.get("label"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_owned();
                let communities = m
                    .get("communities")
                    .or_else(|| m.get("community"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                c.as_str()
                                    .map(str::to_owned)
                                    .or_else(|| Some(c.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Section {
                    id,
                    name,
                    communities,
                }
            })
            .collect()
    } else {
        archetypes::derive_sections_from_communities(&nodes, &labels, &lang, opts.max_sections)
    };

    let sections = builder::normalize_sections(&sections, &lang);
    let report_text = loader::load_report(Some(&paths.report));

    if nodes.is_empty() {
        return Err(HtmlError::EmptyGraph);
    }
    if sections.len() <= 1 {
        return Err(HtmlError::NoSections);
    }

    meta.insert(
        "project_name".to_owned(),
        serde_json::Value::String(loader::infer_project_name(&paths.graph, &meta)),
    );
    meta.insert(
        "node_count".to_owned(),
        serde_json::Value::Number(nodes.len().into()),
    );
    meta.insert(
        "edge_count".to_owned(),
        serde_json::Value::Number(edges.len().into()),
    );
    meta.insert(
        "hyperedge_count".to_owned(),
        serde_json::Value::Number(hyperedges.len().into()),
    );

    let output_path = if let Some(ref out) = opts.output {
        let p = PathBuf::from(out);
        if p.is_absolute() {
            p
        } else {
            paths.base.join(p)
        }
    } else {
        let project_name = meta
            .get("project_name")
            .and_then(|v| v.as_str())
            .unwrap_or("project");
        paths.graphify_out.join(format!(
            "{}-callflow.html",
            loader::safe_filename(project_name)
        ))
    };

    let comm_idx = builder::build_community_index(&nodes);
    meta.insert(
        "community_count".to_owned(),
        serde_json::Value::Number(comm_idx.len().into()),
    );

    let section_nodes_map = builder::build_section_node_map(&sections, &comm_idx);
    let classified = builder::classify_edges(&edges, &section_nodes_map, &nodes);

    let project_name = meta
        .get("project_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Project");
    let lang_str = lang.as_str();
    let doc_title = if loader::is_zh(lang_str) {
        format!("{project_name} — 完整调用流程与架构文档")
    } else {
        format!("{project_name} — Complete Call Flow & Architecture Documentation")
    };

    let mut html = String::new();

    // Doctype and head.
    let _ = write!(
        html,
        "<!DOCTYPE html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>{}</title>\n<script src=\"https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js\"></script>\n<style>\n{}\n</style>\n</head>\n<body>\n<div class=\"container\">\n",
        htmlescape::encode_attribute(lang_str),
        htmlescape::encode_minimal(&doc_title),
        template::CSS,
    );

    html.push_str(&render::generate_header(&sections, &meta, lang_str));

    // Architecture Overview.
    let overview_name = sections
        .first()
        .map_or("Architecture Overview", |s| s.name.as_str());
    let _ = write!(
        html,
        "<!-- ====== Architecture Overview ====== -->\n<h2 id=\"overview\">1. {}</h2>\n\n<div class=\"mermaid\">\n",
        htmlescape::encode_minimal(overview_name)
    );
    html.push_str(&diagram::generate_overview_graph(
        &sections,
        &section_nodes_map,
        &classified,
        &edges,
        lang_str,
        opts.diagram_scale,
    ));
    html.push_str("\n</div>\n");
    html.push_str(&render::generate_overview_cards(
        &meta,
        &report_text,
        &sections,
        &section_nodes_map,
        &classified,
        &edges,
        lang_str,
    ));
    let report_card = render::report_highlights(&report_text, lang_str);
    if !report_card.is_empty() {
        let _ = write!(html, "\n<div class=\"grid\">\n  {report_card}\n</div>");
    }
    html.push_str("\n<hr>\n");

    // Per-section content.
    let mut section_num = 1usize;
    for sec in &sections {
        if sec.id == "overview" {
            continue;
        }
        section_num += 1;
        let sid = &sec.id;

        let sec_node_indices = section_nodes_map
            .get(sid.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let sec_nodes: Vec<Node> = sec_node_indices.iter().map(|&i| nodes[i].clone()).collect();
        let sec_edge_indices = classified
            .intra
            .get(sid.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let sec_edges: Vec<CfEdge> = sec_edge_indices.iter().map(|&i| edges[i].clone()).collect();

        render::emit_section_html(
            &mut html,
            sec,
            section_num,
            &sec_nodes,
            &sec_edges,
            lang_str,
            opts.diagram_scale,
            opts.max_diagram_nodes,
            opts.max_diagram_edges,
        );
    }

    // Hyperedges section.
    if !hyperedges.is_empty() {
        html.push_str(
            "<h2 id=\"hyperedges\">Group Relationships (Hyperedges)</h2>\n<div class=\"grid\">\n",
        );
        for he in hyperedges.iter().take(9) {
            let hid = he.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let hlabel = he.get("label").and_then(|v| v.as_str()).unwrap_or(hid);
            let hnodes: Vec<&serde_json::Value> = he
                .get("nodes")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            let hrel = he.get("relation").and_then(|v| v.as_str()).unwrap_or("");
            let _ = write!(
                html,
                "  <div class=\"card\">\n    <h4>{}</h4>\n    <p><code>{}</code> — {} participants</p>\n    <ul>",
                htmlescape::encode_minimal(hlabel),
                htmlescape::encode_minimal(hrel),
                hnodes.len()
            );
            for hn in hnodes.iter().take(5) {
                let _ = write!(
                    html,
                    "\n      <li><code>{}</code></li>",
                    htmlescape::encode_minimal(&hn.to_string())
                );
            }
            if hnodes.len() > 5 {
                let _ = write!(html, "\n      <li>... and {} more</li>", hnodes.len() - 5);
            }
            html.push_str("\n    </ul>\n  </div>");
        }
        html.push_str("\n</div>\n<hr>\n");
    }

    // Statistics section.
    let total_sections = sections.iter().filter(|s| s.id != "overview").count();
    let extracted_count = edges.iter().filter(|e| e.confidence == "EXTRACTED").count();
    let inferred_count = edges.iter().filter(|e| e.confidence == "INFERRED").count();
    let ambiguous_count = edges.iter().filter(|e| e.confidence == "AMBIGUOUS").count();
    let _ = write!(
        html,
        r#"<h2 id="stats">Project Statistics</h2>

<div class="grid">
  <div class="card">
    <h4>Graph</h4>
    <table style="width:100%;font-size:0.85rem;">
      <tr><td>Nodes</td><td>{}</td></tr>
      <tr><td>Edges</td><td>{}</td></tr>
      <tr><td>Hyperedges</td><td>{}</td></tr>
      <tr><td>Communities</td><td>{}</td></tr>
      <tr><td>Documented Sections</td><td>{total_sections}</td></tr>
    </table>
  </div>
  <div class="card">
    <h4>Edge Confidence</h4>
    <table style="width:100%;font-size:0.85rem;">
      <tr><td>EXTRACTED</td><td>{extracted_count}</td></tr>
      <tr><td>INFERRED</td><td>{inferred_count}</td></tr>
      <tr><td>AMBIGUOUS</td><td>{ambiguous_count}</td></tr>
    </table>
  </div>
</div>
"#,
        nodes.len(),
        edges.len(),
        hyperedges.len(),
        comm_idx.len(),
    );

    // Footer.
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
    let _ = write!(
        html,
        "<div style=\"text-align:center; padding:40px 0; color: var(--muted); font-size:0.9rem;\">\n  <p>{} — Architecture Documentation</p>\n  <p>Generated: {} · graphify callflow-html</p>\n</div>\n",
        htmlescape::encode_minimal(project_name),
        now,
    );

    // Close.
    html.push_str("</div><!-- .container -->\n\n");
    html.push_str(template::JS_FOOTER);
    html.push_str("\n\n</body>\n</html>");

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, html.as_bytes())?;
    Ok(output_path)
}
