//! Mermaid diagram generators for the architecture overview and per-section
//! call-flow charts.
//!
//! Extracted so that the Mermaid-specific layout logic (node selection,
//! subgraph grouping, edge scoring) is separate from both the structural
//! analysis (`builder`) and the surrounding HTML rendering (`render`).

use std::collections::HashMap;
use std::path::Path;

use indexmap::IndexMap;

use super::builder::{
    ClassifiedEdges, edge_score, humanize_label, node_kind, preferred_edges, relation_label,
    section_edge_summary,
};
use super::loader::{mermaid_section_id, node_mermaid_id, safe_file_path, safe_mermaid_text};
use super::options::{CfEdge, Node, Section};

// ── Mermaid init helpers ────────────────────────────────────────────────────

/// Build the `%%{init: ...}%%` preamble and `flowchart <direction>` line for Mermaid.
///
/// `scale` adjusts font size, node spacing, and rank spacing proportionally and
/// is clamped to `[0.65, 1.8]`. Mirrors Python `_mermaid_init` in `callflow.py`.
pub(super) fn mermaid_init(scale: f64, direction: &str) -> String {
    let scale = scale.clamp(0.65_f64, 1.8_f64);
    // Build the Mermaid init JSON to match Python's json.dumps output.
    let font_size = format!("{:.1}px", 15.0 * scale);
    // Use f64::round() then convert; values are small positive so truncation is safe.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let node_spacing = (48.0 * scale).round() as u64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rank_spacing = (64.0 * scale).round() as u64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let padding = (14.0 * scale).round() as u64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let diagram_padding = (10.0 * scale).round() as u64;
    let config = serde_json::json!({
        "theme": "dark",
        "themeVariables": {
            "fontSize": font_size,
            "fontFamily": "Segoe UI, system-ui, sans-serif",
            "primaryColor": "#1e293b",
            "primaryTextColor": "#e2e8f0",
            "primaryBorderColor": "#38bdf8",
            "secondaryColor": "#0f172a",
            "tertiaryColor": "#334155",
            "lineColor": "#64748b",
            "textColor": "#e2e8f0"
        },
        "flowchart": {
            "htmlLabels": true,
            "curve": "basis",
            "nodeSpacing": node_spacing,
            "rankSpacing": rank_spacing,
            "padding": padding,
            "diagramPadding": diagram_padding,
            "useMaxWidth": true
        }
    });
    let config_str = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_owned()); // infallible for well-formed Value
    format!("%%{{init: {config_str}}}%%\nflowchart {direction}")
}

/// Return the static list of Mermaid `classDef` lines for node type styling.
pub(super) fn mermaid_class_defs() -> Vec<&'static str> {
    vec![
        "    classDef entry fill:#422006,stroke:#fbbf24,color:#fde68a,stroke-width:1px;",
        "    classDef api fill:#450a0a,stroke:#f87171,color:#fee2e2,stroke-width:1px;",
        "    classDef async fill:#2e1065,stroke:#a78bfa,color:#ede9fe,stroke-width:1px;",
        "    classDef klass fill:#064e3b,stroke:#34d399,color:#d1fae5,stroke-width:1px;",
        "    classDef ui fill:#831843,stroke:#f472b6,color:#fce7f3,stroke-width:1px;",
        "    classDef module fill:#172554,stroke:#60a5fa,color:#dbeafe,stroke-width:1px;",
        "    classDef test fill:#3f3f46,stroke:#a1a1aa,color:#f4f4f5,stroke-width:1px;",
        "    classDef concept fill:#292524,stroke:#a8a29e,color:#fafaf9,stroke-dasharray:4 3;",
        "    classDef function fill:#0f172a,stroke:#38bdf8,color:#e0f2fe,stroke-width:1px;",
    ]
}

// ── Node selection for diagrams ─────────────────────────────────────────────

/// Accumulate weighted edge scores per node to rank nodes by connectivity.
fn node_degree_scores<'a>(edges: &'a [&'a CfEdge]) -> HashMap<&'a str, f64> {
    let mut scores: HashMap<&'a str, f64> = HashMap::new();
    for e in edges {
        let s = edge_score(e);
        *scores.entry(e.source.as_str()).or_insert(0.0) += s;
        *scores.entry(e.target.as_str()).or_insert(0.0) += s;
    }
    scores
}

/// Select up to `max_nodes` nodes for the diagram, ranked by weighted connectivity.
///
/// Scores nodes by summing `edge_score` for all preferred edges they participate
/// in. Nodes connected by at least one edge are preferred over isolated ones.
pub(super) fn select_diagram_nodes<'a>(
    nodes: &'a [Node],
    edges: &[CfEdge],
    max_nodes: usize,
) -> Vec<&'a Node> {
    let node_by_id: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let pref1 = preferred_edges(edges, false);
    let usable_edges: Vec<&CfEdge> = if pref1.is_empty() {
        preferred_edges(edges, true)
    } else {
        pref1
    };
    let scores = node_degree_scores(&usable_edges);
    let mut outgoing: HashMap<&str, usize> = HashMap::new();
    let mut incoming: HashMap<&str, usize> = HashMap::new();
    for e in &usable_edges {
        *outgoing.entry(e.source.as_str()).or_insert(0) += 1;
        *incoming.entry(e.target.as_str()).or_insert(0) += 1;
    }

    let mut selected: Vec<&Node> = vec![];
    // Use owned String keys to avoid lifetime conflicts with the closure.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    macro_rules! add_node_macro {
        ($nid:expr) => {{
            let nid: &str = $nid;
            if seen.contains(nid) {
                false
            } else if let Some(&node) = node_by_id.get(nid) {
                if node_kind(node) == "concept" && selected.len() >= max_nodes.max(4) / 3 {
                    false
                } else {
                    selected.push(node);
                    seen.insert(nid.to_owned());
                    selected.len() >= max_nodes
                }
            } else {
                false
            }
        }};
    }

    // Entry candidates: nodes that call out more than they are called.
    let mut entry_candidates: Vec<&str> = node_by_id.keys().copied().collect();
    entry_candidates.sort_by(|&a, &b| {
        let a_out = outgoing.get(a).copied().unwrap_or(0);
        let a_in = incoming.get(a).copied().unwrap_or(0);
        let b_out = outgoing.get(b).copied().unwrap_or(0);
        let b_in = incoming.get(b).copied().unwrap_or(0);
        // Prefer nodes that call out more than they receive (entry points).
        let diff_cmp = (b_out.saturating_sub(b_in)).cmp(&(a_out.saturating_sub(a_in)));
        diff_cmp.then(b_out.cmp(&a_out)).then(a.cmp(b))
    });

    let take = (max_nodes.max(3) / 3).max(3);
    for &nid in entry_candidates.iter().take(take) {
        if *outgoing.get(nid).unwrap_or(&0) > 0 && add_node_macro!(nid) {
            return selected;
        }
    }

    // Pull in strongest neighbors.
    let mut sorted_edges: Vec<&CfEdge> = usable_edges.clone();
    sorted_edges.sort_by(|a, b| {
        edge_score(b)
            .partial_cmp(&edge_score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for e in &sorted_edges {
        if add_node_macro!(e.source.as_str()) {
            return selected;
        }
        if add_node_macro!(e.target.as_str()) {
            return selected;
        }
    }

    // Fallback: sort all nodes.
    let mut all_sorted: Vec<&Node> = nodes.iter().collect();
    all_sorted.sort_by(|a, b| {
        let ak = usize::from(node_kind(a) == "concept");
        let bk = usize::from(node_kind(b) == "concept");
        let a_score = scores.get(a.id.as_str()).copied().unwrap_or(0.0);
        let b_score = scores.get(b.id.as_str()).copied().unwrap_or(0.0);
        ak.cmp(&bk)
            .then(
                b_score
                    .partial_cmp(&a_score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.id.cmp(&b.id))
    });
    for node in all_sorted {
        if !seen.contains(node.id.as_str()) {
            selected.push(node);
            seen.insert(node.id.clone());
        }
        if selected.len() >= max_nodes {
            break;
        }
    }
    selected
}

/// Produce a Mermaid-safe label string for a node.
fn node_label_mermaid(node: &Node) -> String {
    let label = humanize_label(&node.label, &node.source_file);
    let source_file = safe_file_path(&node.source_file);
    if !source_file.is_empty()
        && !label.ends_with(
            Path::new(&source_file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(""),
        )
    {
        format!(
            "{}<br/><small>{}</small>",
            safe_mermaid_text(&label),
            safe_mermaid_text(&source_file)
        )
    } else {
        safe_mermaid_text(&label)
    }
}

/// Group a node slice by their `source_file` path, preserving insertion order.
fn group_nodes_by_file<'a>(nodes: &[&'a Node]) -> IndexMap<String, Vec<&'a Node>> {
    let mut groups: IndexMap<String, Vec<&Node>> = IndexMap::new();
    for &node in nodes {
        let sf = if node.source_file.is_empty() {
            "External / generated".to_owned()
        } else {
            safe_file_path(&node.source_file)
        };
        groups.entry(sf).or_default().push(node);
    }
    // Sort: largest group first, then alphabetically.
    let mut vec: Vec<(String, Vec<&Node>)> = groups.into_iter().collect();
    vec.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    vec.into_iter().collect()
}

// ── Public diagram generators ───────────────────────────────────────────────

/// Generate the architecture overview Mermaid diagram.
#[must_use]
pub(super) fn generate_overview_graph(
    sections: &[Section],
    section_nodes_map: &IndexMap<String, Vec<usize>>,
    classified: &ClassifiedEdges,
    edges: &[CfEdge],
    lang: &str,
    diagram_scale: f64,
) -> String {
    let mut lines = vec![mermaid_init(diagram_scale, "LR")];
    let section_defs: Vec<&Section> = sections.iter().filter(|s| s.id != "overview").collect();

    for sec in &section_defs {
        let sid = mermaid_section_id(&sec.id);
        let node_count = section_nodes_map.get(&sec.id).map_or(0, Vec::len);
        let label = format!(
            "{}<br/><small>{} {}</small>",
            safe_mermaid_text(sec.name.as_str()),
            node_count,
            safe_mermaid_text("nodes")
        );
        lines.push(format!("    {sid}(\"{label}\")"));
        lines.push(format!("    class {sid} module;"));
    }

    let aggregated = section_edge_summary(classified, edges);
    let mut agg_sorted: Vec<_> = aggregated.iter().collect();
    agg_sorted.sort_by_key(|b| std::cmp::Reverse(b.1.0));
    for ((src, tgt), (count, relation)) in agg_sorted.iter().take(12) {
        let src_id = mermaid_section_id(src);
        let tgt_id = mermaid_section_id(tgt);
        let mut lbl = relation_label(relation, lang);
        if *count > 1 {
            lbl = format!("{lbl} x{count}");
        }
        lines.push(format!("    {src_id} -->|{lbl}| {tgt_id}"));
    }

    if aggregated.is_empty() && section_defs.len() > 1 {
        for (prev, cur) in section_defs.iter().zip(section_defs.iter().skip(1)) {
            lines.push(format!(
                "    {} -.-> {}",
                mermaid_section_id(&prev.id),
                mermaid_section_id(&cur.id)
            ));
        }
    }

    lines.extend(mermaid_class_defs().iter().map(|s| (*s).to_owned()));
    lines.join("\n")
}

/// Parameters for [`generate_section_flowchart`].
pub(super) struct FlowchartParams<'a> {
    pub(super) section_id: &'a str,
    pub(super) section_name: &'a str,
    pub(super) nodes: &'a [Node],
    pub(super) edges: &'a [CfEdge],
    pub(super) lang: &'a str,
    pub(super) diagram_scale: f64,
    pub(super) max_nodes: usize,
    pub(super) max_edges: usize,
}

/// Generate a compact call-flow chart for a single section.
#[must_use]
pub(super) fn generate_section_flowchart(p: &FlowchartParams<'_>) -> String {
    use super::loader::pick_text;

    let section_id = p.section_id;
    let section_name = p.section_name;
    let nodes = p.nodes;
    let edges = p.edges;
    let lang = p.lang;
    let diagram_scale = p.diagram_scale;
    let max_nodes = p.max_nodes;
    let max_edges = p.max_edges;
    let mut lines = vec![mermaid_init(diagram_scale, "LR")];
    lines.push(format!(
        "    %% Section: {} ({} nodes, {} edges)",
        safe_mermaid_text(section_name),
        nodes.len(),
        edges.len()
    ));

    if nodes.is_empty() {
        let empty_zh = format!("{section_name} - 无节点");
        let empty_en = format!("{section_name} - no nodes");
        let empty_label = pick_text(lang, &empty_zh, &empty_en);
        lines.push(format!("    empty(\"{}\")", safe_mermaid_text(empty_label)));
        lines.extend(mermaid_class_defs().iter().map(|s| (*s).to_owned()));
        return lines.join("\n");
    }

    let selected = select_diagram_nodes(nodes, edges, max_nodes);
    let selected_ids: std::collections::HashSet<&str> =
        selected.iter().map(|n| n.id.as_str()).collect();

    let visible_edges: Vec<&CfEdge> = {
        let pref = preferred_edges(edges, false)
            .into_iter()
            .filter(|e| {
                selected_ids.contains(e.source.as_str()) && selected_ids.contains(e.target.as_str())
            })
            .collect::<Vec<_>>();
        if pref.is_empty() {
            preferred_edges(edges, true)
                .into_iter()
                .filter(|e| {
                    selected_ids.contains(e.source.as_str())
                        && selected_ids.contains(e.target.as_str())
                })
                .collect()
        } else {
            pref
        }
    };

    let groups = group_nodes_by_file(&selected);
    let mut class_lines: Vec<String> = vec![];
    for (source_file, group) in &groups {
        let group_id = node_mermaid_id(&format!("{section_id}_{source_file}"));
        let indent = if groups.len() > 1 && group.len() > 1 {
            lines.push(format!(
                "    subgraph {group_id}[\"{}\"]",
                safe_mermaid_text(source_file)
            ));
            "        "
        } else {
            "    "
        };
        for node in group {
            let mid = node_mermaid_id(&node.id);
            lines.push(format!("{indent}{mid}(\"{}\")", node_label_mermaid(node)));
            class_lines.push(format!("    class {mid} {};", node_kind(node)));
        }
        if groups.len() > 1 && group.len() > 1 {
            lines.push("    end".to_owned());
        }
    }

    let mut sorted_edges: Vec<&CfEdge> = visible_edges.clone();
    sorted_edges.sort_by(|a, b| {
        edge_score(b)
            .partial_cmp(&edge_score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut included = 0usize;
    for e in &sorted_edges {
        if included >= max_edges {
            break;
        }
        let src_id = node_mermaid_id(&e.source);
        let tgt_id = node_mermaid_id(&e.target);
        let rel = relation_label(&e.relation, lang);
        lines.push(format!("    {src_id} -->|{rel}| {tgt_id}"));
        included += 1;
    }

    let omitted_nodes = nodes.len().saturating_sub(selected.len());
    let omitted_edges = visible_edges.len().saturating_sub(included);
    if omitted_nodes > 0 || omitted_edges > 0 {
        lines.push(format!(
            "    %% Omitted for readability: {omitted_nodes} nodes, {omitted_edges} edges"
        ));
    }
    lines.extend(class_lines);
    lines.extend(mermaid_class_defs().iter().map(|s| (*s).to_owned()));
    lines.join("\n")
}
