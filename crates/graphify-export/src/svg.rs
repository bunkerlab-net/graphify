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

// ── Spring layout (Fruchterman-Reingold, seeded deterministically) ────────────

/// Compute a spring-layout for the graph nodes.
///
/// Returns a map from `node_id` to (x, y) in `[0, 1] × [0, 1]`.
///
/// Mirrors Python `nx.spring_layout(G, seed=42, k=2/√n)`.
#[allow(clippy::too_many_lines)] // Inherent complexity of a layout algorithm
fn spring_layout(graph: &Graph) -> IndexMap<String, (f64, f64)> {
    let nodes: Vec<&String> = graph.node_map.keys().collect();
    let n = nodes.len();
    if n == 0 {
        return IndexMap::new();
    }

    // Initial positions: arrange on a circle (deterministic, seed-free)
    let mut pos: Vec<f64> = Vec::with_capacity(n * 2);
    for i in 0..n {
        #[allow(clippy::cast_precision_loss)]
        let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        pos.push(angle.cos());
        pos.push(angle.sin());
    }

    // Build adjacency list using node indices
    let node_idx: IndexMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, &id)| (id.as_str(), i))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in graph.edges() {
        if let (Some(&ui), Some(&vi)) = (
            node_idx.get(edge.source.as_str()),
            node_idx.get(edge.target.as_str()),
        ) {
            if !adj[ui].contains(&vi) {
                adj[ui].push(vi);
            }
            if !adj[vi].contains(&ui) {
                adj[vi].push(ui);
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let k = if n > 1 { 2.0 / (n as f64).sqrt() } else { 1.0 };
    let iterations = 50_usize;
    let t_initial = 0.1_f64; // area / 10

    for iter in 0..iterations {
        #[allow(clippy::cast_precision_loss)]
        let t = t_initial * (1.0 - iter as f64 / iterations as f64);

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

        // Apply displacements, clipped to temperature
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

    // Normalise to [0, 1]
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
#[allow(clippy::too_many_lines)] // Inherent complexity of SVG rendering logic
pub fn to_svg(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_path: &Path,
    community_labels: Option<&IndexMap<i64, String>>,
    figsize: (u32, u32),
) -> Result<(), ExportError> {
    let node_community = node_community_map(communities);
    let layout = spring_layout(graph);

    // Compute degrees
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
    let max_deg = degree.values().copied().max().unwrap_or(1).max(1);

    // SVG canvas dimensions
    let margin: u32 = 60;
    let w = figsize.0 * 100;
    let h = figsize.1 * 100;
    let inner_w = w - 2 * margin;
    let inner_h = h - 2 * margin;

    let margin_f = f64::from(margin);
    let plot_w = f64::from(inner_w);
    let plot_h = f64::from(inner_h);

    let to_svg_x = |x: f64| margin_f + x * plot_w;
    let to_svg_y = |y: f64| margin_f + y * plot_h;

    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" style=\"background:#1a1a2e\">"
    );

    // Draw edges
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
            to_svg_x(pos_u.0),
            to_svg_y(pos_u.1),
            to_svg_x(pos_v.0),
            to_svg_y(pos_v.1),
        );
    }

    // Draw nodes
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
            to_svg_x(x),
            to_svg_y(y),
            size,
        );
    }

    // Draw labels for high-degree nodes
    for (node_id, attrs) in graph.nodes() {
        let deg = degree.get(node_id).copied().unwrap_or(1);
        #[allow(clippy::cast_precision_loss)]
        let threshold = max_deg as f64 * 0.15;
        #[allow(clippy::cast_precision_loss)]
        if deg as f64 >= threshold {
            let (x, y) = layout.get(node_id).copied().unwrap_or((0.5, 0.5));
            let label = attrs
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(node_id);
            let label_esc = label
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;");
            let _ = writeln!(
                svg,
                "  <text x=\"{:.1}\" y=\"{:.1}\" fill=\"white\" font-size=\"7\" text-anchor=\"middle\">{label_esc}</text>",
                to_svg_x(x),
                to_svg_y(y) - 8.0,
            );
        }
    }

    // Legend
    if let Some(cl) = community_labels {
        let legend_x: u32 = 10;
        let mut legend_y: u32 = 10;
        for (cid, label) in cl {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let color = COMMUNITY_COLORS[(cid.unsigned_abs() as usize) % COMMUNITY_COLORS.len()];
            let n = communities.get(cid).map_or(0, Vec::len);
            let label_esc = label
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
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

    svg.push_str("</svg>\n");
    std::fs::write(output_path, svg)?;
    Ok(())
}
