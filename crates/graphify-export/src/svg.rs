//! SVG export — `to_svg`.
//!
//! Mirrors Python `to_svg` from `graphify-py/graphify/export.py`.
//!
//! The Python implementation uses `matplotlib` + `networkx.spring_layout`
//! (seed=42, k=2/√n). Since neither library is available in Rust, we
//! implement an equivalent Fruchterman-Reingold spring layout in pure Rust,
//! then emit SVG via string generation.
//!
//! Visual output will not be pixel-identical to the Python version; the layout
//! algorithm and font rendering differ. However all structural properties
//! (node colours by community, dashed edges for non-EXTRACTED, legend) match.

use std::fmt::Write as FmtWrite;
use std::path::Path;

use graphify_build::Graph;
use indexmap::IndexMap;
use serde_json::Value;

use crate::{COMMUNITY_COLORS, ExportError, node_community_map};

fn svg_text_escape(s: &str) -> String {
    htmlescape::encode_minimal(s)
}

// ── Spring layout (Fruchterman-Reingold, seeded deterministically) ────────────

/// Compute a spring-layout for the graph nodes.
///
/// Returns a map from `node_id` to (x, y) in `[0, 1] × [0, 1]`.
///
/// Mirrors Python `nx.spring_layout(G, seed=42, k=2/√n)`.
fn spring_layout(graph: &Graph) -> IndexMap<String, (f64, f64)> {
    let nodes: Vec<&String> = graph.node_map.keys().collect();
    let n = nodes.len();
    if n == 0 {
        return IndexMap::new();
    }

    let mut pos = initial_circle_positions(n);
    let adj = build_adjacency_indices(&nodes, graph);

    #[allow(clippy::cast_precision_loss)]
    let k = if n > 1 { 2.0 / (n as f64).sqrt() } else { 1.0 };
    let iterations = 50_usize;
    let t_initial = 0.1_f64; // area / 10
    for iter in 0..iterations {
        #[allow(clippy::cast_precision_loss)]
        let t = t_initial * (1.0 - iter as f64 / iterations as f64);
        let (disp_x, disp_y) = compute_layout_forces(&pos, &adj, n, k);
        apply_displacements(&mut pos, &disp_x, &disp_y, t, n);
    }

    normalise_positions(&pos, &nodes)
}

/// Place `n` nodes around the unit circle so the layout starts from a deterministic seed.
fn initial_circle_positions(n: usize) -> Vec<f64> {
    let mut pos: Vec<f64> = Vec::with_capacity(n * 2);
    for i in 0..n {
        #[allow(clippy::cast_precision_loss)]
        let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        pos.push(angle.cos());
        pos.push(angle.sin());
    }
    pos
}

/// Build an undirected adjacency list keyed by `nodes` index.
fn build_adjacency_indices(nodes: &[&String], graph: &Graph) -> Vec<Vec<usize>> {
    let n = nodes.len();
    let node_idx: IndexMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, &id)| (id.as_str(), i))
        .collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in graph.edges() {
        let (Some(&ui), Some(&vi)) = (
            node_idx.get(edge.source.as_str()),
            node_idx.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if !adj[ui].contains(&vi) {
            adj[ui].push(vi);
        }
        if !adj[vi].contains(&ui) {
            adj[vi].push(ui);
        }
    }
    adj
}

/// One iteration of Fruchterman-Reingold force computation. Returns per-node
/// displacement vectors.
fn compute_layout_forces(
    pos: &[f64],
    adj: &[Vec<usize>],
    n: usize,
    k: f64,
) -> (Vec<f64>, Vec<f64>) {
    let mut disp_x = vec![0.0_f64; n];
    let mut disp_y = vec![0.0_f64; n];
    // Repulsive forces
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = pos[2 * i] - pos[2 * j];
            let dy = pos[2 * i + 1] - pos[2 * j + 1];
            let dist = (dx * dx + dy * dy).sqrt().max(1e-10);
            let force = (k * k) / dist;
            let nx = (dx / dist) * force;
            let ny = (dy / dist) * force;
            disp_x[i] += nx;
            disp_y[i] += ny;
            disp_x[j] -= nx;
            disp_y[j] -= ny;
        }
    }
    // Attractive forces along edges
    for i in 0..n {
        for &j in &adj[i] {
            if j > i {
                let dx = pos[2 * i] - pos[2 * j];
                let dy = pos[2 * i + 1] - pos[2 * j + 1];
                let dist = (dx * dx + dy * dy).sqrt().max(1e-10);
                let force = (dist * dist) / k;
                let nx = (dx / dist) * force;
                let ny = (dy / dist) * force;
                disp_x[i] -= nx;
                disp_y[i] -= ny;
                disp_x[j] += nx;
                disp_y[j] += ny;
            }
        }
    }
    (disp_x, disp_y)
}

/// Apply per-node displacements clipped to temperature `t`, then clamp into `[-2, 2]`.
fn apply_displacements(pos: &mut [f64], disp_x: &[f64], disp_y: &[f64], t: f64, n: usize) {
    for i in 0..n {
        let mag = (disp_x[i] * disp_x[i] + disp_y[i] * disp_y[i])
            .sqrt()
            .max(1e-10);
        let scale = mag.min(t) / mag;
        pos[2 * i] += disp_x[i] * scale;
        pos[2 * i + 1] += disp_y[i] * scale;
        pos[2 * i] = pos[2 * i].clamp(-2.0, 2.0);
        pos[2 * i + 1] = pos[2 * i + 1].clamp(-2.0, 2.0);
    }
}

/// Normalise the iterated positions into `[0, 1] × [0, 1]`.
fn normalise_positions(pos: &[f64], nodes: &[&String]) -> IndexMap<String, (f64, f64)> {
    let min_x = pos.iter().step_by(2).copied().fold(f64::INFINITY, f64::min);
    let max_x = pos
        .iter()
        .step_by(2)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = pos
        .iter()
        .skip(1)
        .step_by(2)
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max_y = pos
        .iter()
        .skip(1)
        .step_by(2)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);

    let range_x = (max_x - min_x).max(1e-10);
    let range_y = (max_y - min_y).max(1e-10);

    let mut layout: IndexMap<String, (f64, f64)> = IndexMap::new();
    for (i, &id) in nodes.iter().enumerate() {
        let x = (pos[2 * i] - min_x) / range_x;
        let y = (pos[2 * i + 1] - min_y) / range_y;
        layout.insert(id.clone(), (x, y));
    }
    layout
}

// ── SVG generation ────────────────────────────────────────────────────────────

/// Export graph as an SVG file.
///
/// Uses a Fruchterman-Reingold spring layout. Node sizes scale with degree.
/// Community colours match the HTML output. Edges are dashed for non-EXTRACTED.
///
/// Mirrors Python `to_svg`.
///
/// # Errors
///
/// Returns [`ExportError::Io`] on file-write failure.
pub fn to_svg(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_path: &Path,
    community_labels: Option<&IndexMap<i64, String>>,
    figsize: (u32, u32),
) -> Result<(), ExportError> {
    let node_community = node_community_map(communities);
    let layout = spring_layout(graph);
    let degree = compute_node_degrees(graph);
    let max_deg = degree.values().copied().max().unwrap_or(1).max(1);

    let canvas = SvgCanvas::from_figsize(figsize);
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" style=\"background:#1a1a2e\">",
        w = canvas.width,
        h = canvas.height,
    );

    draw_edges(&mut svg, graph, &layout, &canvas);
    draw_nodes(
        &mut svg,
        graph,
        &layout,
        &node_community,
        &degree,
        max_deg,
        &canvas,
    );
    draw_node_labels(&mut svg, graph, &layout, &degree, max_deg, &canvas);
    if let Some(cl) = community_labels {
        draw_legend(&mut svg, cl, communities);
    }
    svg.push_str("</svg>\n");
    std::fs::write(output_path, svg)?;
    Ok(())
}

/// SVG plot dimensions + plot-to-canvas projection.
struct SvgCanvas {
    width: u32,
    height: u32,
    margin_f: f64,
    plot_w: f64,
    plot_h: f64,
}

impl SvgCanvas {
    fn from_figsize(figsize: (u32, u32)) -> Self {
        let margin: u32 = 60;
        let width = figsize.0 * 100;
        let height = figsize.1 * 100;
        Self {
            width,
            height,
            margin_f: f64::from(margin),
            plot_w: f64::from(width - 2 * margin),
            plot_h: f64::from(height - 2 * margin),
        }
    }

    fn to_x(&self, x: f64) -> f64 {
        self.margin_f + x * self.plot_w
    }
    fn to_y(&self, y: f64) -> f64 {
        self.margin_f + y * self.plot_h
    }
}

/// Total in+out degree per node (self-loops count once).
fn compute_node_degrees(graph: &Graph) -> IndexMap<String, usize> {
    let mut degree: IndexMap<String, usize> = IndexMap::new();
    for (id, _) in graph.nodes() {
        degree.insert(id.clone(), 0);
    }
    for edge in graph.edges() {
        *degree.entry(edge.source.clone()).or_insert(0) += 1;
        if edge.source != edge.target {
            *degree.entry(edge.target.clone()).or_insert(0) += 1;
        }
    }
    degree
}

fn draw_edges(
    svg: &mut String,
    graph: &Graph,
    layout: &IndexMap<String, (f64, f64)>,
    canvas: &SvgCanvas,
) {
    for edge in graph.edges() {
        let pos_u = layout.get(&edge.source).copied().unwrap_or((0.5, 0.5));
        let pos_v = layout.get(&edge.target).copied().unwrap_or((0.5, 0.5));
        let confidence = edge
            .attrs
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("EXTRACTED");
        let (dash_attr, alpha) = if confidence == "EXTRACTED" {
            ("", "0.6")
        } else {
            (" stroke-dasharray=\"6,4\"", "0.3")
        };
        let _ = writeln!(
            svg,
            "  <line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" \
             stroke=\"#aaaaaa\" stroke-width=\"0.8\" opacity=\"{alpha}\"{dash_attr}/>",
            canvas.to_x(pos_u.0),
            canvas.to_y(pos_u.1),
            canvas.to_x(pos_v.0),
            canvas.to_y(pos_v.1),
        );
    }
}

fn draw_nodes(
    svg: &mut String,
    graph: &Graph,
    layout: &IndexMap<String, (f64, f64)>,
    node_community: &IndexMap<String, i64>,
    degree: &IndexMap<String, usize>,
    max_deg: usize,
    canvas: &SvgCanvas,
) {
    for (node_id, _attrs) in graph.nodes() {
        let (x, y) = layout.get(node_id).copied().unwrap_or((0.5, 0.5));
        let cid = node_community.get(node_id).copied().unwrap_or(0);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let color = COMMUNITY_COLORS[(cid.unsigned_abs() as usize) % COMMUNITY_COLORS.len()];
        let deg = degree.get(node_id).copied().unwrap_or(1);
        #[allow(clippy::cast_precision_loss)]
        let size = 5.0 + 20.0 * (deg as f64 / max_deg as f64);
        let _ = writeln!(
            svg,
            "  <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.1}\" fill=\"{color}\" opacity=\"0.9\"/>",
            canvas.to_x(x),
            canvas.to_y(y),
            size,
        );
    }
}

fn draw_node_labels(
    svg: &mut String,
    graph: &Graph,
    layout: &IndexMap<String, (f64, f64)>,
    degree: &IndexMap<String, usize>,
    max_deg: usize,
    canvas: &SvgCanvas,
) {
    #[allow(clippy::cast_precision_loss)]
    let threshold = max_deg as f64 * 0.15;
    for (node_id, attrs) in graph.nodes() {
        let deg = degree.get(node_id).copied().unwrap_or(1);
        #[allow(clippy::cast_precision_loss)]
        let deg_f = deg as f64;
        if deg_f < threshold {
            continue;
        }
        let (x, y) = layout.get(node_id).copied().unwrap_or((0.5, 0.5));
        let label = attrs
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(node_id);
        let label_esc = svg_text_escape(label);
        let _ = writeln!(
            svg,
            "  <text x=\"{:.1}\" y=\"{:.1}\" fill=\"white\" font-size=\"7\" text-anchor=\"middle\">{label_esc}</text>",
            canvas.to_x(x),
            canvas.to_y(y) - 8.0,
        );
    }
}

fn draw_legend(
    svg: &mut String,
    cl: &IndexMap<i64, String>,
    communities: &IndexMap<i64, Vec<String>>,
) {
    let legend_x: u32 = 10;
    let mut legend_y: u32 = 10;
    for (cid, label) in cl {
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let color = COMMUNITY_COLORS[(cid.unsigned_abs() as usize) % COMMUNITY_COLORS.len()];
        let n = communities.get(cid).map_or(0, Vec::len);
        let label_esc = svg_text_escape(label);
        let _ = writeln!(
            svg,
            "  <rect x=\"{legend_x}\" y=\"{legend_y}\" width=\"12\" height=\"12\" fill=\"{color}\" rx=\"6\"/>"
        );
        let _ = writeln!(
            svg,
            "  <text x=\"{}\" y=\"{}\" fill=\"white\" font-size=\"8\">{label_esc} ({n})</text>",
            legend_x + 16,
            legend_y + 10,
        );
        legend_y += 18;
    }
}
