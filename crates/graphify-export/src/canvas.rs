//! Obsidian Canvas export — `to_canvas`.
//!
//! Mirrors Python `to_canvas` from `graphify-py/graphify/export.py`.
//!
//! Produces a structured layout: communities arranged in a grid, nodes within
//! each community arranged in rows of 3. Edges shown between connected nodes,
//! capped at 200 highest-weight.

use std::path::Path;

use graphify_build::Graph;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Value, json};

use crate::ExportError;

// Obsidian canvas colour codes (cycle through for communities)
const CANVAS_COLORS: [&str; 6] = ["1", "2", "3", "4", "5", "6"]; // red, orange, yellow, green, cyan, purple

#[allow(clippy::expect_used)]
static UNSAFE_CHARS_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"[\\/*?:"<>|#^\[\]]"#).expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
static MD_EXT_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)\.(md|mdx|qmd|markdown)$").expect("literal pattern is valid")
});

/// Sanitize a node label for use as an Obsidian Canvas node title.
///
/// Strips filesystem-unsafe characters and trailing Markdown extensions.
fn safe_name(label: &str) -> String {
    let cleaned = label.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let cleaned = UNSAFE_CHARS_RE.replace_all(&cleaned, "");
    let cleaned = cleaned.trim().to_string();
    let cleaned = MD_EXT_RE.replace(&cleaned, "").into_owned();
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        crate::util::cap_filename(&cleaned)
    }
}

/// Export graph as an Obsidian Canvas file.
///
/// Communities are laid out in a grid; nodes within each community are arranged
/// in rows of 3. Edges between nodes are included, capped at 200 highest-weight.
///
/// Mirrors Python `to_canvas`.
///
/// # Errors
///
/// Returns [`ExportError::Io`] or [`ExportError::Json`] on failure.
pub fn to_canvas(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_path: &Path,
    community_labels: Option<&IndexMap<i64, String>>,
    node_filenames: Option<&IndexMap<String, String>>,
) -> Result<(), ExportError> {
    let owned_filenames: IndexMap<String, String>;
    let node_filenames: &IndexMap<String, String> = if let Some(nf) = node_filenames {
        nf
    } else {
        owned_filenames = derive_node_filenames(graph);
        &owned_filenames
    };

    let sorted_cids: Vec<i64> = {
        let mut v: Vec<i64> = communities.keys().copied().collect();
        v.sort_unstable();
        v
    };
    let group_layout = compute_group_layout(communities, &sorted_cids);

    let all_canvas_nodes: indexmap::IndexSet<String> = communities
        .values()
        .flat_map(|members| members.iter().cloned())
        .collect();

    let mut canvas_nodes: Vec<Value> = Vec::new();
    for (idx, &cid) in sorted_cids.iter().enumerate() {
        let emit_ctx = CommunityEmit {
            graph,
            members: &communities[&cid],
            community_labels,
            node_filenames,
        };
        emit_community_nodes(&emit_ctx, cid, idx, group_layout[&cid], &mut canvas_nodes);
    }
    let canvas_edges = emit_canvas_edges(graph, &all_canvas_nodes);

    let canvas_data = json!({ "nodes": canvas_nodes, "edges": canvas_edges });
    std::fs::write(output_path, serde_json::to_string_pretty(&canvas_data)?)?;
    Ok(())
}

/// Derive per-node filenames (same dedup logic as `to_obsidian`).
fn derive_node_filenames(graph: &Graph) -> IndexMap<String, String> {
    let mut nf: IndexMap<String, String> = IndexMap::new();
    let mut seen_names: IndexMap<String, usize> = IndexMap::new();
    for (node_id, attrs) in graph.nodes() {
        let raw_label = attrs
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(node_id);
        let base = safe_name(raw_label);
        let fname = if let Some(count) = seen_names.get_mut(&base) {
            *count += 1;
            format!("{base}_{count}")
        } else {
            seen_names.insert(base.clone(), 0);
            base.clone()
        };
        nf.insert(node_id.clone(), fname);
    }
    nf
}

/// Compute per-community `(x, y, width, height)` rectangles in a grid layout.
fn compute_group_layout(
    communities: &IndexMap<i64, Vec<String>>,
    sorted_cids: &[i64],
) -> IndexMap<i64, (usize, usize, usize, usize)> {
    let num_communities = communities.len();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let cols = if num_communities > 0 {
        (num_communities as f64).sqrt().ceil() as usize
    } else {
        1
    };
    let rows = if num_communities > 0 {
        num_communities.div_ceil(cols)
    } else {
        1
    };

    let group_sizes: IndexMap<i64, (usize, usize)> = sorted_cids
        .iter()
        .map(|&cid| {
            let n = communities.get(&cid).map_or(0, Vec::len);
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let w = if n > 0 {
                (220.0 * (n as f64).sqrt().ceil()) as usize
            } else {
                0
            }
            .max(600);
            let h = if n > 0 { 100 * n.div_ceil(3) + 120 } else { 0 }.max(400);
            (cid, (w, h))
        })
        .collect();

    let mut col_widths: Vec<usize> = vec![0; cols];
    let mut row_heights: Vec<usize> = vec![0; rows];
    for (linear, &cid) in sorted_cids.iter().enumerate() {
        let col_idx = linear % cols;
        let row_idx = linear / cols;
        let (w, h) = group_sizes[&cid];
        col_widths[col_idx] = col_widths[col_idx].max(w);
        row_heights[row_idx] = row_heights[row_idx].max(h);
    }

    let gap: usize = 80;
    let mut group_layout: IndexMap<i64, (usize, usize, usize, usize)> = IndexMap::new();
    for (linear, &cid) in sorted_cids.iter().enumerate() {
        let col_idx = linear % cols;
        let row_idx = linear / cols;
        let gx = col_widths[..col_idx].iter().sum::<usize>() + col_idx * gap;
        let gy = row_heights[..row_idx].iter().sum::<usize>() + row_idx * gap;
        let (gw, gh) = group_sizes[&cid];
        group_layout.insert(cid, (gx, gy, gw, gh));
    }
    group_layout
}

/// Per-community emit context for [`emit_community_nodes`].
struct CommunityEmit<'a> {
    graph: &'a Graph,
    members: &'a [String],
    community_labels: Option<&'a IndexMap<i64, String>>,
    node_filenames: &'a IndexMap<String, String>,
}

/// Emit the group node + per-member file nodes for one community.
fn emit_community_nodes(
    ctx: &CommunityEmit<'_>,
    cid: i64,
    idx: usize,
    rect: (usize, usize, usize, usize),
    canvas_nodes: &mut Vec<Value>,
) {
    let CommunityEmit {
        graph,
        members,
        community_labels,
        node_filenames,
    } = *ctx;
    let (gx, gy, gw, gh) = rect;
    let community_name = community_labels
        .and_then(|cl| cl.get(&cid))
        .cloned()
        .unwrap_or_else(|| format!("Community {cid}"));
    let canvas_color = CANVAS_COLORS[idx % CANVAS_COLORS.len()];
    canvas_nodes.push(json!({
        "id": format!("g{cid}"),
        "type": "group",
        "label": community_name,
        "x": gx,
        "y": gy,
        "width": gw,
        "height": gh,
        "color": canvas_color,
    }));

    let mut sorted_members = members.to_vec();
    sorted_members.sort_by_key(|n| {
        graph
            .node_data(n)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(n)
            .to_string()
    });
    for (m_idx, node_id) in sorted_members.iter().enumerate() {
        let col = m_idx % 3;
        let row = m_idx / 3;
        let nx_x = gx + 20 + col * (180 + 20);
        let nx_y = gy + 80 + row * (60 + 20);
        let fname = node_filenames.get(node_id).cloned().unwrap_or_else(|| {
            safe_name(
                graph
                    .node_data(node_id)
                    .and_then(|a| a.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(node_id),
            )
        });
        canvas_nodes.push(json!({
            "id": format!("n_{node_id}"),
            "type": "file",
            "file": format!("{fname}.md"),
            "x": nx_x,
            "y": nx_y,
            "width": 180,
            "height": 60,
        }));
    }
}

/// Emit canvas edges between nodes in the canvas, capped at the 200 highest-weight.
fn emit_canvas_edges(graph: &Graph, all_canvas_nodes: &indexmap::IndexSet<String>) -> Vec<Value> {
    let mut all_edges_weighted: Vec<(f64, &str, &str, String)> = Vec::new();
    for edge in graph.edges() {
        if !all_canvas_nodes.contains(&edge.source) || !all_canvas_nodes.contains(&edge.target) {
            continue;
        }
        let weight = edge
            .attrs
            .get("weight")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        let relation = edge
            .attrs
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("");
        let conf = edge
            .attrs
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("EXTRACTED");
        let label = if relation.is_empty() {
            format!("[{conf}]")
        } else {
            format!("{relation} [{conf}]")
        };
        all_edges_weighted.push((weight, &edge.source, &edge.target, label));
    }
    all_edges_weighted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    all_edges_weighted
        .into_iter()
        .take(200)
        .map(|(_, u, v, label)| {
            json!({
                "id": format!("e_{u}_{v}"),
                "fromNode": format!("n_{u}"),
                "toNode": format!("n_{v}"),
                "label": label,
            })
        })
        .collect()
}
