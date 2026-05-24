//! HTML fragment generators: navigation bar, section headers, call-detail
//! tables, overview cards, and the report-highlights sidebar card.
//!
//! Extracted so the HTML-emission logic is separate from both the structural
//! analysis (`builder`) and the page-assembly entry point (`mod`).
//!
//! Per-node HTML helpers (tag badges, descriptions, node refs) live in the
//! sibling `render_node` module.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use indexmap::IndexMap;

use super::archetypes::section_keywords;
use super::builder::{
    ClassifiedEdges, node_kind, relation_label, section_edge_summary, should_include_edge,
};
use super::loader::{html_comment_text, is_zh, pick_text, safe_file_path};
use super::options::{CfEdge, Node, Section};
use super::render_node::{describe_node, format_node_refs, suggest_tag};

// ── Navigation ──────────────────────────────────────────────────────────────

/// Render the sticky navigation bar linking to each section anchor.
pub(super) fn generate_nav(sections: &[Section]) -> String {
    let links: Vec<String> = sections
        .iter()
        .map(|sec| {
            format!(
                "    <a href=\"#{}\">{}</a>",
                htmlescape::encode_attribute(&sec.id),
                htmlescape::encode_minimal(&sec.name)
            )
        })
        .collect();
    format!("<div class=\"nav\">\n{}\n</div>", links.join("\n"))
}

// ── Table / card generators ─────────────────────────────────────────────────

/// Render `<tr>` rows for the call-detail table of a section.
///
/// Each row shows one node, its tag badge, a description, and lists of upstream
/// (callers) and downstream (callees) node refs.
pub(super) fn generate_call_table_rows(
    nodes: &[Node],
    section_edges: &[CfEdge],
    lang: &str,
) -> String {
    if nodes.is_empty() {
        return String::new();
    }
    let mut upstream: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut downstream: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in section_edges {
        if ["calls", "imports", "imports_from", "uses", "method"].contains(&e.relation.as_str()) {
            upstream
                .entry(e.target.as_str())
                .or_default()
                .push(e.source.as_str());
            downstream
                .entry(e.source.as_str())
                .or_default()
                .push(e.target.as_str());
        }
    }

    let mut rows = String::new();
    for (i, n) in nodes.iter().enumerate().take(30) {
        let nid = n.id.as_str();
        let label = &n.label;
        let source_file = safe_file_path(&n.source_file);
        let file_type = &n.file_type;
        let tag = suggest_tag(label, file_type, lang, node_kind(n));

        let incoming: Vec<&str> = upstream
            .get(nid)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .to_vec();
        let outgoing: Vec<&str> = downstream
            .get(nid)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .to_vec();

        // Deduplicate while preserving order.
        let uniq_incoming: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            incoming.into_iter().filter(|&id| seen.insert(id)).collect()
        };
        let uniq_outgoing: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            outgoing.into_iter().filter(|&id| seen.insert(id)).collect()
        };

        let in_text = format_node_refs(
            &uniq_incoming,
            nodes,
            lang,
            pick_text(
                lang,
                "外部入口 / 无直接入边",
                "External entry / no inbound edge",
            ),
            3,
        );
        let out_text = format_node_refs(
            &uniq_outgoing,
            nodes,
            lang,
            pick_text(lang, "无直接出边", "No direct outbound edge"),
            3,
        );
        let _ = write!(
            rows,
            "<tr>\n  <td>{}</td>\n  <td><code>{}</code><br><small style=\"color:var(--muted)\">{}</small></td>\n  <td>{}</td>\n  <td>{}</td>\n  <td>{}</td>\n  <td>{}</td>\n</tr>\n",
            i + 1,
            htmlescape::encode_minimal(label),
            htmlescape::encode_minimal(&source_file),
            tag,
            in_text,
            out_text,
            htmlescape::encode_minimal(&describe_node(label, &source_file, file_type, lang)),
        );
    }
    rows
}

/// Render the page `<h1>` title, subtitle stats line, and navigation bar.
pub(super) fn generate_header(
    sections: &[Section],
    meta: &IndexMap<String, serde_json::Value>,
    lang: &str,
) -> String {
    let project_name = meta
        .get("project_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Project");
    let commit = meta
        .get("built_at_commit")
        .and_then(|v| v.as_str())
        .map_or("unknown", |s| &s[..s.len().min(7)]);
    let node_count = meta
        .get("node_count")
        .map_or_else(|| "?".to_owned(), std::string::ToString::to_string);
    let edge_count = meta
        .get("edge_count")
        .map_or_else(|| "?".to_owned(), std::string::ToString::to_string);
    let community_count = meta
        .get("community_count")
        .map_or_else(|| "?".to_owned(), std::string::ToString::to_string);

    let (title, subtitle) = if is_zh(lang) {
        (
            format!("{project_name} — 完整调用流程与架构文档"),
            format!(
                "由 graphify 知识图谱生成：{node_count} 个节点、{edge_count} 条边、{community_count} 个社区。Commit: {commit}"
            ),
        )
    } else {
        (
            format!("{project_name} — Complete Call Flow & Architecture Documentation"),
            format!(
                "Generated from graphify knowledge graph: {node_count} nodes, {edge_count} edges, {community_count} communities. Commit: {commit}"
            ),
        )
    };

    format!(
        "<h1>{}</h1>\n<p class=\"subtitle\">{}</p>\n\n{}\n",
        htmlescape::encode_minimal(&title),
        htmlescape::encode_minimal(&subtitle),
        generate_nav(sections),
    )
}

/// Derive a linear call-flow chain from the inter-section edge summary.
///
/// Attempts to order sections by their heaviest outgoing edges, producing
/// a readable left-to-right flow for the overview card.
fn derive_flow_chain(
    sections: &[Section],
    classified: &ClassifiedEdges,
    edges: &[CfEdge],
) -> String {
    let section_names: HashMap<&str, &str> = sections
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str()))
        .collect();
    let order: Vec<&str> = sections
        .iter()
        .filter(|s| s.id != "overview")
        .map(|s| s.id.as_str())
        .collect();
    if order.is_empty() {
        return "Graph nodes -> documentation".to_owned();
    }

    let aggregated = section_edge_summary(classified, edges);
    let mut outgoing: HashMap<&str, Vec<(&str, usize)>> = HashMap::new();
    let mut incoming: HashMap<&str, usize> = HashMap::new();
    for ((src, tgt), (count, _)) in &aggregated {
        outgoing
            .entry(src.as_str())
            .or_default()
            .push((tgt.as_str(), *count));
        *incoming.entry(tgt.as_str()).or_insert(0) += count;
    }

    let start = *order
        .iter()
        .min_by_key(|&&sid| {
            (
                incoming.get(sid).copied().unwrap_or(0),
                order.iter().position(|&x| x == sid).unwrap_or(0),
            )
        })
        .unwrap_or(&order[0]);

    let mut chain = vec![start];
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::from([start]);
    let mut current = start;

    let limit = 7.min(order.len());
    while chain.len() < limit {
        let nxt = if let Some(candidates) = outgoing.get(current) {
            let filtered: Vec<(&str, usize)> = candidates
                .iter()
                .copied()
                .filter(|(t, _)| !seen.contains(t))
                .collect();
            filtered.into_iter().max_by_key(|(_, c)| *c).map(|(t, _)| t)
        } else {
            None
        };

        let nxt = nxt.or_else(|| order.iter().find(|&&sid| !seen.contains(sid)).copied());
        match nxt {
            Some(n) => {
                chain.push(n);
                seen.insert(n);
                current = n;
            }
            None => break,
        }
    }
    chain
        .iter()
        .map(|&sid| *section_names.get(sid).unwrap_or(&sid))
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// Render the overview section cards (one per architecture section) plus the flow chain.
///
/// `_meta` and `_report_text` are accepted for API symmetry with the Python
/// reference and reserved for future enrichment; they are intentionally
/// unused in the current implementation.
pub(super) fn generate_overview_cards(
    _meta: &IndexMap<String, serde_json::Value>,
    _report_text: &str,
    sections: &[Section],
    section_nodes_map: &IndexMap<String, Vec<usize>>,
    classified: &ClassifiedEdges,
    edges: &[CfEdge],
    lang: &str,
) -> String {
    let rows: Vec<String> = sections
        .iter()
        .filter(|s| s.id != "overview")
        .map(|sec| {
            let communities = sec
                .communities
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            let node_count = section_nodes_map.get(&sec.id).map_or(0, Vec::len);
            format!(
                "<tr><td>{}</td><td>{node_count}</td><td><code>{}</code></td></tr>",
                htmlescape::encode_minimal(&sec.name),
                htmlescape::encode_minimal(&communities)
            )
        })
        .collect();

    let flow = derive_flow_chain(sections, classified, edges);
    let layer_title = pick_text(lang, "架构层次", "Architecture Layers");
    let layer_cols = pick_text(
        lang,
        "<tr><th>层</th><th>节点</th><th>社区</th></tr>",
        "<tr><th>Layer</th><th>Nodes</th><th>Communities</th></tr>",
    );
    let flow_title = pick_text(lang, "核心数据流", "Core Flow");
    format!(
        r#"<div class="grid">
  <div class="card">
    <h4>{layer_title}</h4>
    <table style="width:100%;font-size:0.85rem;">
      {layer_cols}
      {}
    </table>
  </div>
  <div class="card">
    <h4>{flow_title}</h4>
    <div class="arrow-chain">{}</div>
  </div>
</div>"#,
        rows.join(""),
        htmlescape::encode_minimal(&flow),
    )
}

/// Render a one-paragraph intro for a section showing node count, top files, and keywords.
pub(super) fn generate_section_intro(
    sec: &Section,
    nodes: &[Node],
    edge_count: usize,
    lang: &str,
) -> String {
    let node_refs: Vec<&Node> = nodes.iter().collect();
    let mut file_counts: HashMap<&str, usize> = HashMap::new();
    for n in nodes {
        if !n.source_file.is_empty() {
            *file_counts.entry(n.source_file.as_str()).or_insert(0) += 1;
        }
    }
    let mut files_sorted: Vec<(&str, usize)> = file_counts.into_iter().collect();
    files_sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    let files: Vec<String> = files_sorted
        .iter()
        .take(3)
        .map(|(p, _)| safe_file_path(p))
        .collect();
    let keywords = section_keywords(&node_refs, 4);
    let text = if is_zh(lang) {
        let file_text = if files.is_empty() {
            "未标注源文件".to_owned()
        } else {
            files.join("、")
        };
        let kw_text = if keywords.is_empty() {
            sec.name.clone()
        } else {
            keywords.join("、")
        };
        format!(
            "{} 汇集了与 {} 相关的实现，主要分布在 {}。本节覆盖 {} 个节点、{} 条内部边，图中只展示最有代表性的调用关系以保持可读性。",
            sec.name,
            kw_text,
            file_text,
            nodes.len(),
            edge_count
        )
    } else {
        let file_text = if files.is_empty() {
            "unmapped files".to_owned()
        } else {
            files.join(", ")
        };
        let kw_text = if keywords.is_empty() {
            sec.name.clone()
        } else {
            keywords.join(", ")
        };
        format!(
            "{} groups implementation around {}, mostly in {}. This section covers {} nodes and {} internal edges; the diagram shows only representative relationships to stay readable.",
            sec.name,
            kw_text,
            file_text,
            nodes.len(),
            edge_count
        )
    };
    format!("<p>{}</p>", htmlescape::encode_minimal(&text))
}

/// Render the stats, design-note, and call-detail cards for a single section.
///
/// `_sec` is currently unused but retained for API symmetry with the Python
/// reference (sections may eventually drive additional copy here).
pub(super) fn generate_section_cards(
    _sec: &Section,
    nodes: &[Node],
    section_edges: &[CfEdge],
    lang: &str,
) -> String {
    let mut file_counts: HashMap<&str, usize> = HashMap::new();
    for n in nodes {
        if !n.source_file.is_empty() {
            *file_counts.entry(n.source_file.as_str()).or_insert(0) += 1;
        }
    }
    let mut top_files: Vec<(&str, usize)> = file_counts.into_iter().collect();
    top_files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let top_files: Vec<(&str, usize)> = top_files.into_iter().take(8).collect();

    let file_rows = if top_files.is_empty() {
        format!(
            "<tr><td colspan=\"2\">{}</td></tr>",
            htmlescape::encode_minimal(pick_text(lang, "无源文件映射", "No source file mapping"))
        )
    } else {
        top_files
            .iter()
            .map(|(path, count)| {
                format!(
                    "<tr><td><code>{}</code></td><td>{} {}</td></tr>",
                    htmlescape::encode_minimal(&safe_file_path(path)),
                    count,
                    htmlescape::encode_minimal(pick_text(lang, "个节点", "nodes"))
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut relation_counts: HashMap<String, usize> = HashMap::new();
    for e in section_edges {
        if should_include_edge(e) {
            *relation_counts.entry(e.relation.clone()).or_insert(0) += 1;
        }
    }
    let mut rel_sorted: Vec<(String, usize)> = relation_counts.into_iter().collect();
    rel_sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    let relation_text = if rel_sorted.is_empty() {
        pick_text(
            lang,
            "未检测到高置信调用边",
            "No high-confidence call edges detected",
        )
        .to_owned()
    } else {
        rel_sorted
            .iter()
            .take(4)
            .map(|(rel, count)| format!("{} x{}", relation_label(rel, lang), count))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let note = if is_zh(lang) {
        format!(
            "本节由 graphify 社区聚类生成。关系概况：{relation_text}。图表优先展示高置信、跨节点调用或使用关系，完整节点清单位于表格中。"
        )
    } else {
        format!(
            "This section comes from graphify community clustering. Relationship summary: {relation_text}. The diagram prioritizes high-confidence calls or usage relationships; the table keeps the broader node inventory."
        )
    };
    let key_files = pick_text(lang, "关键文件", "Key Files");
    let role = pick_text(lang, "覆盖节点", "Coverage");
    let design_notes = pick_text(lang, "设计备注", "Design Notes");
    format!(
        r#"<div class="grid">
  <div class="card">
    <h4>{key_files}</h4>
    <table style="width:100%;font-size:0.85rem;">
      <tr><th>File</th><th>{role}</th></tr>
      {file_rows}
    </table>
  </div>
  <div class="card">
    <h4>{design_notes}</h4>
    <p>{}</p>
  </div>
</div>"#,
        htmlescape::encode_minimal(&note)
    )
}

/// Extract and render the most notable bullet-point highlights from `GRAPH_REPORT.md`.
///
/// Parses markdown list items and limits output to the top 6 lines.
#[allow(clippy::expect_used)] // reason: static literal regex cannot fail
pub(super) fn report_highlights(report_text: &str, lang: &str) -> String {
    if report_text.trim().is_empty() {
        return String::new();
    }

    let re_numbered = regex::Regex::new(r"^\d+\.").expect("static regex literal cannot fail");
    let mut keep: Vec<String> = vec![];
    let mut in_gods = false;
    let mut in_summary = false;
    for line in report_text.lines() {
        let stripped = line.trim();
        if let Some(rest) = stripped.strip_prefix("## ") {
            in_summary = rest == "Summary";
            in_gods = stripped.starts_with("## God Nodes");
            continue;
        }
        if in_summary && stripped.starts_with("- ") {
            keep.push(stripped[2..].to_owned());
        } else if in_gods && re_numbered.is_match(stripped) {
            keep.push(stripped.to_owned());
        }
        if keep.len() >= 6 {
            break;
        }
    }

    if keep.is_empty() {
        return String::new();
    }
    let title = pick_text(lang, "图谱报告摘要", "Graph Report Highlights");
    let items: String = keep
        .iter()
        .map(|item| format!("      <li>{}</li>", htmlescape::encode_minimal(item)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<div class="card">
    <h4>{title}</h4>
    <ul>
{items}
    </ul>
  </div>"#
    )
}

/// Inputs for [`emit_section_html`].
pub(super) struct SectionEmit<'a> {
    /// The section being rendered.
    pub sec: &'a Section,
    /// 1-based display number for the section heading (e.g. `2` renders as `"2. <name>"`).
    pub section_num: usize,
    /// Nodes belonging to this section.
    pub sec_nodes: &'a [Node],
    /// Intra-section edges for this section.
    pub sec_edges: &'a [CfEdge],
    /// BCP 47 language tag used for localized labels.
    pub lang: &'a str,
    /// Mermaid diagram scale factor; clamped to `[0.65, 1.8]`.
    pub diagram_scale: f64,
    /// Maximum nodes to render in the section flowchart.
    pub max_diagram_nodes: usize,
    /// Maximum edges to render in the section flowchart.
    pub max_diagram_edges: usize,
}

/// Emit the per-section HTML block into `html`.
pub(super) fn emit_section_html(html: &mut String, args: &SectionEmit<'_>) {
    use super::diagram::{FlowchartParams, generate_section_flowchart};

    let SectionEmit {
        sec,
        section_num,
        sec_nodes,
        sec_edges,
        lang,
        diagram_scale,
        max_diagram_nodes,
        max_diagram_edges,
    } = *args;
    let sid = &sec.id;
    let name = &sec.name;
    let edge_count = sec_edges.len();

    let h3_title = pick_text(lang, "调用明细", "Call Details");
    let number_header = "#";
    let function_header = pick_text(lang, "节点", "Node");
    let type_header = pick_text(lang, "类型", "Type");
    let inbound_header = pick_text(lang, "调用方", "Caller");
    let outbound_header = pick_text(lang, "被调用/依赖", "Callees");
    let desc_header = pick_text(lang, "说明", "Description");

    let _ = write!(
        html,
        "<!-- ====== {section_num}. {} ====== -->\n<h2 id=\"{}\">{section_num}. {}</h2>\n{}\n\n<div class=\"mermaid\">\n{}\n</div>\n\n<h3>{h3_title}</h3>\n<table class=\"call-table\">\n<tr>\n  <th style=\"width:5%\">{number_header}</th>\n  <th style=\"width:28%\">{function_header}</th>\n  <th style=\"width:10%\">{type_header}</th>\n  <th style=\"width:17%\">{inbound_header}</th>\n  <th style=\"width:20%\">{outbound_header}</th>\n  <th style=\"width:20%\">{desc_header}</th>\n</tr>\n{}</table>\n\n{}\n<hr>\n",
        html_comment_text(name),
        htmlescape::encode_attribute(sid),
        htmlescape::encode_minimal(name),
        generate_section_intro(sec, sec_nodes, edge_count, lang),
        generate_section_flowchart(&FlowchartParams {
            section_id: sid,
            section_name: name,
            nodes: sec_nodes,
            edges: sec_edges,
            lang,
            diagram_scale,
            max_nodes: max_diagram_nodes,
            max_edges: max_diagram_edges,
        }),
        generate_call_table_rows(sec_nodes, sec_edges, lang),
        generate_section_cards(sec, sec_nodes, sec_edges, lang),
    );
}
