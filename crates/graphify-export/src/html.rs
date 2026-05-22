//! HTML export — interactive vis.js visualization.
//!
//! Mirrors Python `to_html` / `generate_html` from `graphify-py/graphify/export.py`.

use std::path::Path;

use graphify_build::Graph;
use graphify_security::sanitize_label;
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::{COMMUNITY_COLORS, ExportError, node_community_map, obsidian_tag, viz_node_limit};

// ── Static HTML / JS fragments ─────────────────────────────────────────────────

fn html_styles() -> &'static str {
    r#"<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: #0f0f1a; color: #e0e0e0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; display: flex; height: 100vh; overflow: hidden; }
  #graph { flex: 1; }
  #sidebar { width: 280px; background: #1a1a2e; border-left: 1px solid #2a2a4e; display: flex; flex-direction: column; overflow: hidden; }
  #search-wrap { padding: 12px; border-bottom: 1px solid #2a2a4e; }
  #search { width: 100%; background: #0f0f1a; border: 1px solid #3a3a5e; color: #e0e0e0; padding: 7px 10px; border-radius: 6px; font-size: 13px; outline: none; }
  #search:focus { border-color: #4E79A7; }
  #search-results { max-height: 140px; overflow-y: auto; padding: 4px 12px; border-bottom: 1px solid #2a2a4e; display: none; }
  .search-item { padding: 4px 6px; cursor: pointer; border-radius: 4px; font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .search-item:hover { background: #2a2a4e; }
  #info-panel { padding: 14px; border-bottom: 1px solid #2a2a4e; min-height: 140px; }
  #info-panel h3 { font-size: 13px; color: #aaa; margin-bottom: 8px; text-transform: uppercase; letter-spacing: 0.05em; }
  #info-content { font-size: 13px; color: #ccc; line-height: 1.6; }
  #info-content .field { margin-bottom: 5px; }
  #info-content .field b { color: #e0e0e0; }
  #info-content .empty { color: #555; font-style: italic; }
  .neighbor-link { display: block; padding: 2px 6px; margin: 2px 0; border-radius: 3px; cursor: pointer; font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; border-left: 3px solid #333; }
  .neighbor-link:hover { background: #2a2a4e; }
  #neighbors-list { max-height: 160px; overflow-y: auto; margin-top: 4px; }
  #legend-wrap { flex: 1; overflow-y: auto; padding: 12px; }
  #legend-wrap h3 { font-size: 13px; color: #aaa; margin-bottom: 10px; text-transform: uppercase; letter-spacing: 0.05em; }
  .legend-item { display: flex; align-items: center; gap: 8px; padding: 4px 0; cursor: pointer; border-radius: 4px; font-size: 12px; }
  .legend-item:hover { background: #2a2a4e; padding-left: 4px; }
  .legend-item.dimmed { opacity: 0.35; }
  .legend-dot { width: 12px; height: 12px; border-radius: 50%; flex-shrink: 0; }
  .legend-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .legend-count { color: #666; font-size: 11px; }
  #stats { padding: 10px 14px; border-top: 1px solid #2a2a4e; font-size: 11px; color: #555; }
  #legend-controls { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; padding: 4px 0; }
  #legend-controls label { display: flex; align-items: center; gap: 6px; cursor: pointer; font-size: 12px; color: #aaa; user-select: none; }
  #legend-controls label:hover { color: #e0e0e0; }
  .legend-cb, #select-all-cb { appearance: none; -webkit-appearance: none; width: 14px; height: 14px; border: 1.5px solid #3a3a5e; border-radius: 3px; background: #0f0f1a; cursor: pointer; position: relative; flex-shrink: 0; }
  .legend-cb:checked, #select-all-cb:checked { background: #4E79A7; border-color: #4E79A7; }
  .legend-cb:checked::after, #select-all-cb:checked::after { content: ''; position: absolute; left: 3.5px; top: 1px; width: 4px; height: 7px; border: solid #fff; border-width: 0 2px 2px 0; transform: rotate(45deg); }
  #select-all-cb:indeterminate { background: #4E79A7; border-color: #4E79A7; }
  #select-all-cb:indeterminate::after { content: ''; position: absolute; left: 2px; top: 5px; width: 8px; height: 2px; background: #fff; border: none; transform: none; }
</style>"#
}

fn hyperedge_script(hyperedges_json: &str) -> String {
    format!(
        r"<script>
// Render hyperedges as shaded regions
const hyperedges = {hyperedges_json};
// afterDrawing passes ctx already transformed to network coordinate space.
// Draw node positions raw — no manual pan/zoom/DPR math needed.
network.on('afterDrawing', function(ctx) {{
    hyperedges.forEach(h => {{
        const positions = h.nodes
            .map(nid => network.getPositions([nid])[nid])
            .filter(p => p !== undefined);
        if (positions.length < 2) return;
        ctx.save();
        ctx.globalAlpha = 0.12;
        ctx.fillStyle = '#6366f1';
        ctx.strokeStyle = '#6366f1';
        ctx.lineWidth = 2;
        ctx.beginPath();
        // Centroid and expanded hull in network coordinates
        const cx = positions.reduce((s, p) => s + p.x, 0) / positions.length;
        const cy = positions.reduce((s, p) => s + p.y, 0) / positions.length;
        const expanded = positions.map(p => ({{
            x: cx + (p.x - cx) * 1.15,
            y: cy + (p.y - cy) * 1.15
        }}));
        ctx.moveTo(expanded[0].x, expanded[0].y);
        expanded.slice(1).forEach(p => ctx.lineTo(p.x, p.y));
        ctx.closePath();
        ctx.fill();
        ctx.globalAlpha = 0.4;
        ctx.stroke();
        // Label
        ctx.globalAlpha = 0.8;
        ctx.fillStyle = '#4f46e5';
        ctx.font = 'bold 11px sans-serif';
        ctx.textAlign = 'center';
        ctx.fillText(h.label, cx, cy - 5);
        ctx.restore();
    }});
}});
</script>"
    )
}

#[allow(clippy::too_many_lines)] // Inherent complexity of large inline JavaScript
fn html_script(nodes_json: &str, edges_json: &str, legend_json: &str) -> String {
    format!(
        r#"<script>
const RAW_NODES = {nodes_json};
const RAW_EDGES = {edges_json};
const LEGEND = {legend_json};

// HTML-escape helper — prevents XSS when injecting graph data into innerHTML
function esc(s) {{
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');
}}

// Build vis datasets
const nodesDS = new vis.DataSet(RAW_NODES.map(n => ({{
  id: n.id, label: n.label, color: n.color, size: n.size,
  font: n.font, title: n.title,
  _community: n.community, _community_name: n.community_name,
  _source_file: n.source_file, _file_type: n.file_type, _degree: n.degree,
}})));

const edgesDS = new vis.DataSet(RAW_EDGES.map((e, i) => ({{
  id: i, from: e.from, to: e.to,
  label: '',
  title: e.title,
  dashes: e.dashes,
  width: e.width,
  color: e.color,
  arrows: {{ to: {{ enabled: true, scaleFactor: 0.5 }} }},
}})));

const container = document.getElementById('graph');
const network = new vis.Network(container, {{ nodes: nodesDS, edges: edgesDS }}, {{
  physics: {{
    enabled: true,
    solver: 'forceAtlas2Based',
    forceAtlas2Based: {{
      gravitationalConstant: -60,
      centralGravity: 0.005,
      springLength: 120,
      springConstant: 0.08,
      damping: 0.4,
      avoidOverlap: 0.8,
    }},
    stabilization: {{ iterations: 200, fit: true }},
  }},
  interaction: {{
    hover: true,
    tooltipDelay: 100,
    hideEdgesOnDrag: true,
    navigationButtons: false,
    keyboard: false,
  }},
  nodes: {{ shape: 'dot', borderWidth: 1.5 }},
  edges: {{ smooth: {{ type: 'continuous', roundness: 0.2 }}, selectionWidth: 3 }},
}});

network.once('stabilizationIterationsDone', () => {{
  network.setOptions({{ physics: {{ enabled: false }} }});
}});

function showInfo(nodeId) {{
  const n = nodesDS.get(nodeId);
  if (!n) return;
  const neighborIds = network.getConnectedNodes(nodeId);
  const neighborItems = neighborIds.map(nid => {{
    const nb = nodesDS.get(nid);
    const color = nb ? nb.color.background : '#555';
    return `<span class="neighbor-link" style="border-left-color:${{esc(color)}}" onclick="focusNode(${{JSON.stringify(nid)}})">${{esc(nb ? nb.label : nid)}}</span>`;
  }}).join('');
  document.getElementById('info-content').innerHTML = `
    <div class="field"><b>${{esc(n.label)}}</b></div>
    <div class="field">Type: ${{esc(n._file_type || 'unknown')}}</div>
    <div class="field">Community: ${{esc(n._community_name)}}</div>
    <div class="field">Source: ${{esc(n._source_file || '-')}}</div>
    <div class="field">Degree: ${{n._degree}}</div>
    ${{neighborIds.length ? `<div class="field" style="margin-top:8px;color:#aaa;font-size:11px">Neighbors (${{neighborIds.length}})</div><div id="neighbors-list">${{neighborItems}}</div>` : ''}}
  `;
}}

function focusNode(nodeId) {{
  network.focus(nodeId, {{ scale: 1.4, animation: true }});
  network.selectNodes([nodeId]);
  showInfo(nodeId);
}}

// Track hovered node — hover detection is more reliable than click params
let hoveredNodeId = null;
network.on('hoverNode', params => {{
  hoveredNodeId = params.node;
  container.style.cursor = 'pointer';
}});
network.on('blurNode', () => {{
  hoveredNodeId = null;
  container.style.cursor = 'default';
}});
container.addEventListener('click', () => {{
  if (hoveredNodeId !== null) {{
    showInfo(hoveredNodeId);
    network.selectNodes([hoveredNodeId]);
  }}
}});
network.on('click', params => {{
  if (params.nodes.length > 0) {{
    showInfo(params.nodes[0]);
  }} else if (hoveredNodeId === null) {{
    document.getElementById('info-content').innerHTML = '<span class="empty">Click a node to inspect it</span>';
  }}
}});

const searchInput = document.getElementById('search');
const searchResults = document.getElementById('search-results');
searchInput.addEventListener('input', () => {{
  const q = searchInput.value.toLowerCase().trim();
  searchResults.innerHTML = '';
  if (!q) {{ searchResults.style.display = 'none'; return; }}
  const matches = RAW_NODES.filter(n => n.label.toLowerCase().includes(q)).slice(0, 20);
  if (!matches.length) {{ searchResults.style.display = 'none'; return; }}
  searchResults.style.display = 'block';
  matches.forEach(n => {{
    const el = document.createElement('div');
    el.className = 'search-item';
    el.textContent = n.label;
    el.style.borderLeft = `3px solid ${{n.color.background}}`;
    el.style.paddingLeft = '8px';
    el.onclick = () => {{
      network.focus(n.id, {{ scale: 1.5, animation: true }});
      network.selectNodes([n.id]);
      showInfo(n.id);
      searchResults.style.display = 'none';
      searchInput.value = '';
    }};
    searchResults.appendChild(el);
  }});
}});
document.addEventListener('click', e => {{
  if (!searchResults.contains(e.target) && e.target !== searchInput)
    searchResults.style.display = 'none';
}});

const hiddenCommunities = new Set();

const selectAllCb = document.getElementById('select-all-cb');

function updateSelectAllState() {{
  const total = LEGEND.length;
  const hidden = hiddenCommunities.size;
  selectAllCb.checked = hidden === 0;
  selectAllCb.indeterminate = hidden > 0 && hidden < total;
}}

function toggleAllCommunities(hide) {{
  document.querySelectorAll('.legend-item').forEach(item => {{
    hide ? item.classList.add('dimmed') : item.classList.remove('dimmed');
  }});
  document.querySelectorAll('.legend-cb').forEach(cb => {{
    cb.checked = !hide;
  }});
  LEGEND.forEach(c => {{
    if (hide) hiddenCommunities.add(c.cid); else hiddenCommunities.delete(c.cid);
  }});
  const updates = RAW_NODES.map(n => ({{ id: n.id, hidden: hide }}));
  nodesDS.update(updates);
  updateSelectAllState();
}}

const legendEl = document.getElementById('legend');
LEGEND.forEach(c => {{
  const item = document.createElement('div');
  item.className = 'legend-item';
  const cb = document.createElement('input');
  cb.type = 'checkbox';
  cb.className = 'legend-cb';
  cb.checked = true;
  cb.addEventListener('change', (e) => {{
    e.stopPropagation();
    if (cb.checked) {{
      hiddenCommunities.delete(c.cid);
      item.classList.remove('dimmed');
    }} else {{
      hiddenCommunities.add(c.cid);
      item.classList.add('dimmed');
    }}
    const updates = RAW_NODES
      .filter(n => n.community === c.cid)
      .map(n => ({{ id: n.id, hidden: !cb.checked }}));
    nodesDS.update(updates);
    updateSelectAllState();
  }});
  item.innerHTML = `<div class="legend-dot" style="background:${{c.color}}"></div>
    <span class="legend-label">${{c.label}}</span>
    <span class="legend-count">${{c.count}}</span>`;
  item.prepend(cb);
  item.onclick = (e) => {{
    if (e.target === cb) return;
    cb.checked = !cb.checked;
    cb.dispatchEvent(new Event('change'));
  }};
  legendEl.appendChild(item);
}});
</script>"#
    )
}

// ── Main export function ──────────────────────────────────────────────────────

/// Generate an interactive vis.js HTML visualization of the graph.
///
/// Raises [`ExportError::TooLargeForViz`] if graph exceeds the node limit and
/// `node_limit` is `None`. If `node_limit` is `Some`, builds an aggregated
/// community meta-graph instead.
///
/// Mirrors Python `to_html`.
///
/// # Errors
///
/// Returns [`ExportError::Io`] on file-write failure, or
/// [`ExportError::TooLargeForViz`] when the graph is too large and no
/// aggregation limit is provided.
#[allow(clippy::too_many_lines)] // Inherent complexity of full HTML graph generation
pub fn to_html(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    output_path: &Path,
    community_labels: Option<&IndexMap<i64, String>>,
    member_counts: Option<&IndexMap<i64, usize>>,
    node_limit: Option<usize>,
) -> Result<(), ExportError> {
    let limit = node_limit.unwrap_or_else(viz_node_limit);

    if graph.node_count() > limit {
        if node_limit.is_some() {
            // Build aggregated community meta-graph
            println!(
                "Graph has {} nodes (above {limit} limit). Building aggregated community view...",
                graph.node_count()
            );
            let node_to_community = node_community_map(communities);
            let mut meta = Graph::new(graphify_build::GraphKind::Graph);
            for (cid, _members) in communities {
                let mut attrs = indexmap::IndexMap::new();
                let label = community_labels
                    .and_then(|cl| cl.get(cid))
                    .cloned()
                    .unwrap_or_else(|| format!("Community {cid}"));
                attrs.insert("label".to_string(), Value::String(label));
                meta.add_node(&cid.to_string(), attrs);
            }

            // Count cross-community edges
            let mut edge_counts: IndexMap<(i64, i64), usize> = IndexMap::new();
            for edge in graph.edges() {
                let cu = node_to_community.get(&edge.source).copied();
                let cv = node_to_community.get(&edge.target).copied();
                if let (Some(cu), Some(cv)) = (cu, cv)
                    && cu != cv
                {
                    let key = (cu.min(cv), cu.max(cv));
                    *edge_counts.entry(key).or_insert(0) += 1;
                }
            }
            for ((cu, cv), w) in &edge_counts {
                let mut attrs = indexmap::IndexMap::new();
                attrs.insert("weight".to_string(), json!(*w));
                attrs.insert(
                    "relation".to_string(),
                    Value::String(format!("{w} cross-community edges")),
                );
                attrs.insert("confidence".to_string(), Value::String("AGGREGATED".into()));
                meta.add_edge(&cu.to_string(), &cv.to_string(), attrs);
            }

            if meta.node_count() <= 1 {
                println!("Single community - aggregated view not useful. Skipping graph.html.");
                return Ok(());
            }

            let meta_communities: IndexMap<i64, Vec<String>> = communities
                .keys()
                .map(|&cid| (cid, vec![cid.to_string()]))
                .collect();
            let mc: IndexMap<i64, usize> = communities
                .iter()
                .map(|(&cid, members)| (cid, members.len()))
                .collect();
            to_html(
                &meta,
                &meta_communities,
                output_path,
                community_labels,
                Some(&mc),
                None,
            )?;
            println!(
                "graph.html written (aggregated: {} community nodes, {} cross-community edges)",
                meta.node_count(),
                meta.edge_count()
            );
            println!("Tip: run with --obsidian for full node-level detail.");
            return Ok(());
        }
        return Err(ExportError::TooLargeForViz {
            nodes: graph.node_count(),
            limit,
        });
    }

    let node_community = node_community_map(communities);
    let degree = compute_degree(graph);
    let max_deg = degree.values().copied().max().unwrap_or(1).max(1);
    let max_mc = member_counts.map_or(1, |mc| mc.values().copied().max().unwrap_or(1).max(1));

    // Build vis.js nodes list
    let mut vis_nodes: Vec<Value> = Vec::new();
    for (node_id, attrs) in graph.nodes() {
        let cid = node_community.get(node_id).copied().unwrap_or(0);
        // COMMUNITY_COLORS len is 10; usize modulo is always in-bounds
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let color = COMMUNITY_COLORS[(cid.unsigned_abs() as usize) % COMMUNITY_COLORS.len()];
        let raw_label = attrs
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(node_id);
        let label = sanitize_label(Some(raw_label));
        let deg = degree.get(node_id).copied().unwrap_or(1);

        let (size, font_size) = if let Some(mc) = member_counts {
            let mc_val = mc.get(&cid).copied().unwrap_or(1);
            #[allow(clippy::cast_precision_loss)]
            let s = 10.0 + 30.0 * (mc_val as f64 / max_mc as f64);
            (s, 12_u32)
        } else {
            #[allow(clippy::cast_precision_loss)]
            let s = 10.0 + 30.0 * (deg as f64 / max_deg as f64);
            #[allow(clippy::cast_precision_loss)]
            let fs: u32 = if deg as f64 >= max_deg as f64 * 0.15 {
                12
            } else {
                0
            };
            (s, fs)
        };

        let community_name = sanitize_label(
            community_labels
                .and_then(|cl| cl.get(&cid))
                .map(String::as_str)
                .or(Some(&format!("Community {cid}"))),
        );
        let source_file = sanitize_label(
            attrs
                .get("source_file")
                .and_then(Value::as_str)
                .or(Some("")),
        );
        let file_type = attrs
            .get("file_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        vis_nodes.push(json!({
            "id": node_id,
            "label": label,
            "color": {
                "background": color,
                "border": color,
                "highlight": {"background": "#ffffff", "border": color}
            },
            "size": round1(size),
            "font": {"size": font_size, "color": "#ffffff"},
            "title": htmlescape::encode_minimal(&label),
            "community": cid,
            "community_name": community_name,
            "source_file": source_file,
            "file_type": file_type,
            "degree": deg,
        }));
    }

    // Build vis.js edges list
    let mut vis_edges: Vec<Value> = Vec::new();
    for edge in graph.edges() {
        let confidence = edge
            .attrs
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("EXTRACTED");
        let relation = edge
            .attrs
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("");
        let true_src = edge
            .attrs
            .get("_src")
            .and_then(Value::as_str)
            .unwrap_or(&edge.source);
        let true_tgt = edge
            .attrs
            .get("_tgt")
            .and_then(Value::as_str)
            .unwrap_or(&edge.target);
        vis_edges.push(json!({
            "from": true_src,
            "to": true_tgt,
            "label": relation,
            "title": htmlescape::encode_minimal(&format!("{relation} [{confidence}]")),
            "dashes": confidence != "EXTRACTED",
            "width": if confidence == "EXTRACTED" { 2 } else { 1 },
            "color": {"opacity": if confidence == "EXTRACTED" { 0.7f64 } else { 0.35f64 }},
            "confidence": confidence,
        }));
    }

    // Build legend data
    let mut legend_data: Vec<Value> = Vec::new();
    if let Some(cl) = community_labels {
        let mut sorted_cids: Vec<i64> = cl.keys().copied().collect();
        sorted_cids.sort_unstable();
        for cid in sorted_cids {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let color = COMMUNITY_COLORS[(cid.unsigned_abs() as usize) % COMMUNITY_COLORS.len()];
            let lbl = htmlescape::encode_minimal(&sanitize_label(cl.get(&cid).map(String::as_str)));
            let n = member_counts
                .and_then(|mc| mc.get(&cid).copied())
                .or_else(|| communities.get(&cid).map(Vec::len))
                .unwrap_or(0);
            legend_data.push(json!({
                "cid": cid,
                "color": color,
                "label": lbl,
                "count": n,
            }));
        }
    }

    // Escape `</script>` sequences in JSON strings so embedded JSON cannot break out
    let js_safe = |v: &Value| -> String {
        serde_json::to_string(v)
            .unwrap_or_else(|_| "[]".to_string())
            .replace("</", "<\\/")
    };

    let nodes_json = js_safe(&Value::Array(vis_nodes));
    let edges_json = js_safe(&Value::Array(vis_edges));
    let legend_json = js_safe(&Value::Array(legend_data));
    let hyperedges_raw = graph
        .graph_attrs
        .get("hyperedges")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    let hyperedges_json = js_safe(&hyperedges_raw);

    let title = htmlescape::encode_minimal(&sanitize_label(output_path.to_str()));
    let n_communities = communities.len();
    let n_nodes = graph.node_count();
    let n_edges = graph.edge_count();
    let stats =
        format!("{n_nodes} nodes &middot; {n_edges} edges &middot; {n_communities} communities");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>graphify - {title}</title>
<script src="https://unpkg.com/vis-network/standalone/umd/vis-network.min.js"></script>
{styles}
</head>
<body>
<div id="graph"></div>
<div id="sidebar">
  <div id="search-wrap">
    <input id="search" type="text" placeholder="Search nodes..." autocomplete="off">
    <div id="search-results"></div>
  </div>
  <div id="info-panel">
    <h3>Node Info</h3>
    <div id="info-content"><span class="empty">Click a node to inspect it</span></div>
  </div>
  <div id="legend-wrap">
    <h3>Communities</h3>
    <div id="legend-controls">
      <label><input type="checkbox" id="select-all-cb" checked onchange="toggleAllCommunities(!this.checked)">Select All</label>
    </div>
    <div id="legend"></div>
  </div>
  <div id="stats">{stats}</div>
</div>
{script}
{hyperedge_script}
</body>
</html>"#,
        styles = html_styles(),
        script = html_script(&nodes_json, &edges_json, &legend_json),
        hyperedge_script = hyperedge_script(&hyperedges_json),
    );

    std::fs::write(output_path, html)?;
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn compute_degree(graph: &Graph) -> IndexMap<String, usize> {
    let mut deg: IndexMap<String, usize> = IndexMap::new();
    for (id, _) in graph.nodes() {
        deg.insert(id.clone(), 0);
    }
    for edge in graph.edges() {
        *deg.entry(edge.source.clone()).or_insert(0) += 1;
        if edge.source != edge.target {
            *deg.entry(edge.target.clone()).or_insert(0) += 1;
        }
    }
    deg
}

/// Round to 1 decimal place (matches Python `round(x, 1)`).
fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Build the community name, falling back gracefully.
fn community_name_for(cid: i64, community_labels: Option<&IndexMap<i64, String>>) -> String {
    community_labels
        .and_then(|cl| cl.get(&cid))
        .cloned()
        .unwrap_or_else(|| format!("Community {cid}"))
}

// Suppress dead-code — helper will be wired up by the CLI once src/main.rs is ported.
#[allow(dead_code)]
fn community_tag(cid: i64, community_labels: Option<&IndexMap<i64, String>>) -> String {
    obsidian_tag(&community_name_for(cid, community_labels))
}
