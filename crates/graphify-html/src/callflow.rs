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

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};

use chrono::Utc;
use indexmap::IndexMap;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::HtmlError;

// ── CSS template (fixed, project-agnostic) ─────────────────────────────────

static CSS: &str = r":root {
  --bg: #0f172a; --surface: #1e293b; --border: #334155;
  --text: #e2e8f0; --muted: #94a3b8; --accent: #38bdf8;
  --warn: #fbbf24; --err: #f87171; --ok: #34d399;
}
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: 'Segoe UI', system-ui, -apple-system, sans-serif; background: var(--bg); color: var(--text); line-height: 1.7; }
.container { max-width: 1200px; margin: 0 auto; padding: 40px 24px; }
h1 { font-size: 2.4rem; margin-bottom: 8px; background: linear-gradient(135deg, var(--accent), #a78bfa); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
h2 { font-size: 1.7rem; margin: 48px 0 16px; padding-bottom: 8px; border-bottom: 2px solid var(--accent); }
h3 { font-size: 1.25rem; margin: 32px 0 12px; color: var(--accent); }
h4 { font-size: 1.05rem; margin: 20px 0 8px; color: var(--warn); }
p { margin: 8px 0; color: var(--muted); }
.subtitle { color: var(--muted); font-size: 1.1rem; margin-bottom: 32px; }
.mermaid { background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 24px; margin: 20px 0; overflow-x: auto; position: relative; }
.mermaid.is-enhanced { padding: 0; overflow: hidden; min-height: 260px; }
.mermaid-viewport { padding: 54px 24px 24px; overflow: hidden; cursor: grab; touch-action: none; min-height: 260px; }
.mermaid-viewport.is-dragging { cursor: grabbing; }
.mermaid-viewport svg { max-width: none !important; height: auto; transform-origin: 0 0; transition: transform 120ms ease; }
.mermaid-toolbar { position: absolute; top: 10px; right: 10px; z-index: 3; display: flex; align-items: center; gap: 6px; padding: 6px; background: rgba(15,23,42,0.92); border: 1px solid var(--border); border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,0.28); }
.mermaid-toolbar button, .mermaid-toolbar .zoom-level { height: 28px; min-width: 32px; border: 1px solid var(--border); border-radius: 6px; background: #1e293b; color: var(--text); font: 600 0.78rem system-ui, sans-serif; display: inline-flex; align-items: center; justify-content: center; }
.mermaid-toolbar button { cursor: pointer; }
.mermaid-toolbar button:hover { border-color: var(--accent); color: var(--accent); }
.mermaid-toolbar .zoom-level { min-width: 52px; color: var(--muted); background: transparent; }
.call-table { width: 100%; border-collapse: collapse; margin: 16px 0; font-size: 0.92rem; }
.call-table th { background: #1a2744; color: var(--accent); text-align: left; padding: 10px 14px; border: 1px solid var(--border); }
.call-table td { padding: 8px 14px; border: 1px solid var(--border); vertical-align: top; }
.call-table tr:nth-child(even) { background: rgba(255,255,255,0.02); }
.tag { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 0.8rem; font-weight: 600; }
.tag-async { background: #7c3aed33; color: #a78bfa; }
.tag-class { background: #05966933; color: var(--ok); }
.tag-func { background: #2563eb33; color: var(--accent); }
.tag-cmd { background: #d9770633; color: var(--warn); }
.tag-endpoint { background: #dc262633; color: var(--err); }
.tag-hook { background: #db277733; color: #f472b6; }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 20px; margin: 16px 0; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: 16px; margin: 16px 0; }
.arrow-chain { font-family: 'Fira Code', monospace; font-size: 0.85rem; color: var(--accent); padding: 10px; background: rgba(56,189,248,0.06); border-radius: 6px; }
code { font-family: 'Fira Code', 'Cascadia Code', monospace; background: rgba(255,255,255,0.06); padding: 1px 6px; border-radius: 3px; font-size: 0.88em; }
ul, ol { margin: 8px 0 8px 24px; color: var(--muted); }
li { margin: 4px 0; }
a { color: var(--accent); }
hr { border: none; border-top: 1px solid var(--border); margin: 40px 0; }
.nav { position: sticky; top: 0; background: var(--bg); z-index: 10; padding: 12px 0; border-bottom: 1px solid var(--border); display: flex; gap: 20px; flex-wrap: wrap; font-size: 0.9rem; }
.nav a { text-decoration: none; }
.nav a:hover { text-decoration: underline; }
@media (max-width: 768px) { .container { padding: 16px; } h1 { font-size: 1.8rem; } }";

// ── JS footer (interactive zoom/pan for every .mermaid block) ───────────────

static JS_FOOTER: &str = r#"<script>
(function () {
  const mermaidConfig = {
    startOnLoad: false,
    theme: 'dark',
    securityLevel: 'loose',
    flowchart: { htmlLabels: true, useMaxWidth: true },
    themeVariables: {
      primaryColor: '#1e293b',
      primaryTextColor: '#e2e8f0',
      primaryBorderColor: '#38bdf8',
      secondaryColor: '#0f172a',
      tertiaryColor: '#334155',
      lineColor: '#64748b',
      textColor: '#e2e8f0',
    }
  };

  mermaid.initialize(mermaidConfig);

  function clamp(value, min, max) {
    return Math.min(max, Math.max(min, value));
  }

  function enhanceMermaidDiagrams() {
    document.querySelectorAll('.mermaid').forEach((container) => {
      if (container.dataset.zoomReady === 'true') return;
      const svg = container.querySelector('svg');
      if (!svg) return;

      container.dataset.zoomReady = 'true';
      container.classList.add('is-enhanced');

      const viewport = document.createElement('div');
      viewport.className = 'mermaid-viewport';
      svg.parentNode.insertBefore(viewport, svg);
      viewport.appendChild(svg);

      const toolbar = document.createElement('div');
      toolbar.className = 'mermaid-toolbar';
      toolbar.innerHTML = [
        '<button type="button" data-action="zoom-out" title="Zoom out">-</button>',
        '<span class="zoom-level" data-role="level">100%</span>',
        '<button type="button" data-action="zoom-in" title="Zoom in">+</button>',
        '<button type="button" data-action="fit" title="Fit width">Fit</button>',
        '<button type="button" data-action="reset" title="Reset view">Reset</button>'
      ].join('');
      container.insertBefore(toolbar, viewport);

      const state = { scale: 1, x: 0, y: 0, dragging: false, startX: 0, startY: 0, originX: 0, originY: 0 };
      const level = toolbar.querySelector('[data-role="level"]');

      function applyTransform() {
        svg.style.transform = `translate(${state.x}px, ${state.y}px) scale(${state.scale})`;
        level.textContent = `${Math.round(state.scale * 100)}%`;
      }

      function zoomBy(delta) {
        state.scale = clamp(state.scale + delta, 0.25, 3);
        applyTransform();
      }

      function reset() {
        state.scale = 1;
        state.x = 0;
        state.y = 0;
        applyTransform();
      }

      function fitWidth() {
        const rawWidth = svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width
          ? svg.viewBox.baseVal.width
          : svg.getBoundingClientRect().width / state.scale;
        if (!rawWidth) {
          reset();
          return;
        }
        state.scale = clamp((viewport.clientWidth - 48) / rawWidth, 0.25, 1.4);
        state.x = 0;
        state.y = 0;
        applyTransform();
      }

      toolbar.addEventListener('click', (event) => {
        const button = event.target.closest('button[data-action]');
        if (!button) return;
        const action = button.dataset.action;
        if (action === 'zoom-in') zoomBy(0.15);
        if (action === 'zoom-out') zoomBy(-0.15);
        if (action === 'fit') fitWidth();
        if (action === 'reset') reset();
      });

      viewport.addEventListener('wheel', (event) => {
        if (!event.ctrlKey && !event.metaKey) return;
        event.preventDefault();
        zoomBy(event.deltaY < 0 ? 0.1 : -0.1);
      }, { passive: false });

      viewport.addEventListener('pointerdown', (event) => {
        if (event.button !== 0) return;
        state.dragging = true;
        state.startX = event.clientX;
        state.startY = event.clientY;
        state.originX = state.x;
        state.originY = state.y;
        viewport.classList.add('is-dragging');
        viewport.setPointerCapture(event.pointerId);
      });

      viewport.addEventListener('pointermove', (event) => {
        if (!state.dragging) return;
        state.x = state.originX + event.clientX - state.startX;
        state.y = state.originY + event.clientY - state.startY;
        applyTransform();
      });

      function endDrag(event) {
        if (!state.dragging) return;
        state.dragging = false;
        viewport.classList.remove('is-dragging');
        if (viewport.hasPointerCapture(event.pointerId)) {
          viewport.releasePointerCapture(event.pointerId);
        }
      }

      viewport.addEventListener('pointerup', endDrag);
      viewport.addEventListener('pointercancel', endDrag);
      applyTransform();
    });
  }

  function renderMermaid() {
    const result = mermaid.run
      ? mermaid.run({ querySelector: '.mermaid' })
      : Promise.resolve();
    Promise.resolve(result)
      .then(enhanceMermaidDiagrams)
      .catch((error) => {
        console.error('Mermaid render failed:', error);
        enhanceMermaidDiagrams();
      });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', renderMermaid);
  } else {
    renderMermaid();
  }
})();
</script>"#;

// ── Data types ──────────────────────────────────────────────────────────────

/// A lightweight normalized graph node used across all callflow helpers.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub community: String,
    pub source_file: String,
    pub node_type: String,
    pub file_type: String,
}

/// A lightweight normalized graph edge.
#[derive(Debug, Clone)]
pub struct CfEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
    pub confidence_score: f64,
}

/// A section definition (id + name + communities).
#[derive(Debug, Clone)]
pub struct Section {
    pub id: String,
    pub name: String,
    pub communities: Vec<String>,
}

/// Options for [`write_callflow_html`].
#[derive(Debug, Clone)]
pub struct CallflowOptions {
    pub project: Option<PathBuf>,
    pub graphify_out: Option<PathBuf>,
    pub graph: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub labels: Option<PathBuf>,
    pub sections: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub lang: String,
    pub max_sections: usize,
    pub diagram_scale: f64,
    pub max_diagram_nodes: usize,
    pub max_diagram_edges: usize,
}

impl Default for CallflowOptions {
    fn default() -> Self {
        Self {
            project: None,
            graphify_out: None,
            graph: None,
            report: None,
            labels: None,
            sections: None,
            output: None,
            lang: "auto".to_owned(),
            max_sections: 15,
            diagram_scale: 1.0,
            max_diagram_nodes: 18,
            max_diagram_edges: 24,
        }
    }
}

// ── Data loading / normalization ────────────────────────────────────────────

fn first_str_val<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a str> {
    for &k in keys {
        if let Some(serde_json::Value::String(s)) = map.get(k)
            && !s.is_empty()
        {
            return Some(s.as_str());
        }
    }
    None
}

fn to_float(v: &serde_json::Value, default: f64) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(default),
        serde_json::Value::String(s) => s.parse().unwrap_or(default),
        _ => default,
    }
}

/// Normalize a raw JSON node object into a [`Node`].
#[must_use]
pub fn normalize_node(raw: &serde_json::Map<String, serde_json::Value>, index: usize) -> Node {
    let id = first_str_val(
        raw,
        &[
            "id",
            "node_id",
            "key",
            "uid",
            "name",
            "qualified_name",
            "fqname",
            "symbol",
        ],
    )
    .map_or_else(|| format!("node_{}", index + 1), str::to_owned);
    let source_file = first_str_val(
        raw,
        &[
            "source_file",
            "file",
            "file_path",
            "filepath",
            "path",
            "module_path",
            "defined_in",
        ],
    )
    .unwrap_or("")
    .to_owned();
    let label = first_str_val(
        raw,
        &[
            "label",
            "display_name",
            "title",
            "name",
            "qualified_name",
            "fqname",
            "symbol",
        ],
    )
    .map_or_else(|| id.clone(), str::to_owned);
    let community_keys = &[
        "community",
        "community_id",
        "cluster",
        "cluster_id",
        "group",
        "group_id",
        "modularity_class",
    ];
    let community = if let Some(s) = first_str_val(raw, community_keys) {
        s.to_owned()
    } else {
        // Community may be stored as an integer — coerce to string.
        community_keys.iter().find_map(|&k| raw.get(k)).map_or_else(
            || "unknown".to_owned(),
            |v| match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => "unknown".to_owned(),
            },
        )
    };
    let node_type = first_str_val(raw, &["node_type", "kind", "type", "category"])
        .unwrap_or("")
        .to_owned();
    let file_type_raw =
        first_str_val(raw, &["file_type", "content_type", "artifact_type"]).unwrap_or("");
    let file_type = if file_type_raw.is_empty() {
        let suffix = Path::new(&source_file)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if matches!(suffix.as_str(), "md" | "mdx" | "rst" | "txt") {
            "document"
        } else {
            "code"
        }
    } else {
        file_type_raw
    }
    .to_owned();

    Node {
        id,
        label,
        community,
        source_file,
        node_type,
        file_type,
    }
}

/// Normalize a raw JSON edge object into a [`CfEdge`], or return `None` if
/// source/target are missing.
#[must_use]
pub fn normalize_edge(
    raw: &serde_json::Map<String, serde_json::Value>,
    index: usize,
) -> Option<CfEdge> {
    let source = endpoint_id(raw, &["source", "src", "from", "from_id", "start", "u"])?;
    let target = endpoint_id(raw, &["target", "dst", "to", "to_id", "end", "v"])?;
    if source.is_empty() || target.is_empty() {
        return None;
    }
    let relation = first_str_val(raw, &["relation", "type", "kind", "label", "predicate"])
        .unwrap_or("relates")
        .to_lowercase();
    let confidence = first_str_val(raw, &["confidence", "evidence", "provenance"])
        .unwrap_or("EXTRACTED")
        .to_uppercase();
    let score = raw
        .get("confidence_score")
        .or_else(|| raw.get("score"))
        .or_else(|| raw.get("weight"))
        .or_else(|| raw.get("probability"))
        .map_or(1.0, |v| to_float(v, 1.0));
    let id = first_str_val(raw, &["id", "edge_id"])
        .map_or_else(|| format!("edge_{}", index + 1), str::to_owned);
    Some(CfEdge {
        id,
        source,
        target,
        relation,
        confidence,
        confidence_score: score,
    })
}

fn endpoint_id(map: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for &k in keys {
        match map.get(k) {
            Some(serde_json::Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(serde_json::Value::Object(obj)) => {
                if let Some(v) =
                    first_str_val(obj, &["id", "node_id", "key", "name", "qualified_name"])
                    && !v.is_empty()
                {
                    return Some(v.to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

type GraphData = (
    Vec<Node>,
    Vec<CfEdge>,
    Vec<serde_json::Value>,
    IndexMap<String, serde_json::Value>,
);

/// Load graph.json. Returns `(nodes, edges, hyperedges, meta)`.
///
/// # Errors
/// Returns [`HtmlError::Io`] on file read error, or [`HtmlError::EmptyGraph`]
/// if the JSON is malformed.
pub fn load_graph(path: &Path) -> Result<GraphData, HtmlError> {
    let text = std::fs::read_to_string(path)?;
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        HtmlError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.to_string(),
        ))
    })?;
    let data_obj = data.as_object().ok_or(HtmlError::EmptyGraph)?;

    let graph_block = data_obj.get("graph").and_then(|v| v.as_object());
    let meta_block = data_obj.get("metadata").and_then(|v| v.as_object());

    // Try node-link format.
    let (raw_nodes, raw_edges) = if let (Some(nodes_arr), _) = (
        data_obj.get("nodes").and_then(|v| v.as_array()),
        data_obj.get("links").or_else(|| data_obj.get("edges")),
    ) {
        let edges_arr = data_obj
            .get("links")
            .or_else(|| data_obj.get("edges"))
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();
        (nodes_arr.as_slice(), edges_arr)
    } else if let Some(gb) = graph_block {
        let n = gb
            .get("nodes")
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let e = gb
            .get("links")
            .or_else(|| gb.get("edges"))
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();
        (n, e)
    } else {
        (&[] as &[serde_json::Value], &[] as &[serde_json::Value])
    };

    let hyperedges: Vec<serde_json::Value> = {
        let he = data_obj
            .get("hyperedges")
            .or_else(|| graph_block.and_then(|gb| gb.get("hyperedges")))
            .or_else(|| data_obj.get("groups"))
            .and_then(|v| v.as_array());
        he.cloned().unwrap_or_default()
    };

    let nodes: Vec<Node> = raw_nodes
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.as_object().map(|m| normalize_node(m, i)))
        .collect();

    let edges: Vec<CfEdge> = raw_edges
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.as_object().and_then(|m| normalize_edge(m, i)))
        .collect();

    // Build meta map.
    let mut meta: IndexMap<String, serde_json::Value> = IndexMap::new();
    if let Some(gb) = graph_block {
        for (k, v) in gb {
            meta.insert(k.clone(), v.clone());
        }
    }
    if let Some(mb) = meta_block {
        for (k, v) in mb {
            meta.insert(k.clone(), v.clone());
        }
    }
    for key in &[
        "built_at_commit",
        "commit",
        "project_name",
        "repo",
        "repository",
        "language_breakdown",
    ] {
        if let Some(v) = data_obj.get(*key)
            && !meta.contains_key(*key)
        {
            meta.insert((*key).to_owned(), v.clone());
        }
    }
    if let Some(commit) = meta.get("commit").cloned()
        && !meta.contains_key("built_at_commit")
    {
        meta.insert("built_at_commit".to_owned(), commit);
    }

    Ok((nodes, edges, hyperedges, meta))
}

/// Load community labels from `.graphify_labels.json`.
#[must_use]
pub fn load_labels(path: Option<&Path>) -> HashMap<String, String> {
    let Some(p) = path else { return HashMap::new() };
    if !p.exists() {
        return HashMap::new();
    }
    let Ok(text) = std::fs::read_to_string(p) else {
        return HashMap::new();
    };
    let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&text) else {
        return HashMap::new();
    };
    // Unwrap nested wrapper keys.
    if let Some(inner) = data.get("labels").and_then(|v| v.as_object()) {
        data = serde_json::Value::Object(inner.clone());
    } else if let Some(inner) = data.get("communities").and_then(|v| v.as_object()) {
        data = serde_json::Value::Object(inner.clone());
    }
    let Some(obj) = data.as_object() else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (k, v) in obj {
        let label = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(m) => first_str_val(m, &["label", "name", "title"])
                .unwrap_or(k.as_str())
                .to_owned(),
            _ => k.clone(),
        };
        out.insert(k.clone(), label);
    }
    out
}

/// Load `GRAPH_REPORT.md` if it exists.
#[must_use]
pub fn load_report(path: Option<&Path>) -> String {
    let Some(p) = path else { return String::new() };
    if !p.exists() {
        return String::new();
    }
    std::fs::read_to_string(p).unwrap_or_default()
}

// ── Mermaid-safe label helpers ──────────────────────────────────────────────

/// Sanitize text for use inside a Mermaid node label.
#[must_use]
pub fn safe_mermaid_text(text: &str) -> String {
    let mut s = text.to_owned();
    s = s.replace('"', "'");
    s = s.replace('`', "");
    s = s.replace('#', "");
    s = s.replace('|', " ");
    s = s.replace(['{', '}'], "");
    s = s
        .replace("->>", " to ")
        .replace("-->", " to ")
        .replace("->", " to ");
    // Collapse whitespace.
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    htmlescape::encode_minimal(&s)
}

/// Keep HTML comments well-formed.
#[must_use]
pub fn html_comment_text(text: &str) -> String {
    text.replace("--", "- -").replace('\n', " ")
}

/// Build a Mermaid-safe ASCII identifier with a sha1 hash suffix.
#[must_use]
pub fn stable_ascii_id(raw: &str, prefix: &str, limit: usize) -> String {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(raw.as_bytes());
        hex::encode(&hasher.finalize()[..4])
    };
    // Replace non-alnum/_ with underscore, collapse runs.
    let slug: String = {
        let mut out = String::new();
        let mut prev_under = false;
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                out.push(ch);
                prev_under = false;
            } else if !prev_under {
                out.push('_');
                prev_under = true;
            }
        }
        out.trim_matches('_').to_owned()
    };
    let slug = if slug.is_empty() {
        prefix.to_owned()
    } else if slug.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("{prefix}_{slug}")
    } else {
        slug
    };
    let trimmed = slug[..slug.len().min(limit)].trim_end_matches('_');
    format!("{trimmed}_{digest}")
}

/// Generate a safe Mermaid node ID from a graph node id.
#[must_use]
pub fn node_mermaid_id(id: &str) -> String {
    stable_ascii_id(id, "node", 48)
}

/// Convert a section ID to a safe uppercase Mermaid ID.
#[must_use]
pub fn mermaid_section_id(section_id: &str) -> String {
    stable_ascii_id(section_id, "section", 48).to_uppercase()
}

/// Return a short, safe display path (last 3 components).
#[must_use]
pub fn safe_file_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 3 {
        parts[parts.len() - 3..].join("/")
    } else {
        path.to_owned()
    }
}

/// Create a conservative filename stem.
///
/// # Panics
/// Never panics; the `expect` is on a static regex literal that is always valid.
#[must_use]
#[allow(clippy::expect_used)] // reason: static literal regex cannot fail
pub fn safe_filename(text: &str) -> String {
    let re = Regex::new(r"[^A-Za-z0-9._-]+").expect("static regex literal cannot fail");
    let stem = re
        .replace_all(text, "-")
        .trim_matches(|c: char| "-._".contains(c))
        .to_owned();
    if stem.is_empty() {
        "project".to_owned()
    } else {
        stem
    }
}

/// Infer project name from graph path / metadata.
#[must_use]
pub fn infer_project_name(graph_path: &Path, meta: &IndexMap<String, serde_json::Value>) -> String {
    if let Some(serde_json::Value::String(s)) = meta.get("project_name")
        && !s.is_empty()
    {
        return s.clone();
    }
    let resolved = std::fs::canonicalize(graph_path).unwrap_or_else(|_| graph_path.to_path_buf());
    if resolved
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("graphify-out")
        && let Some(name) = resolved
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
    {
        return name.to_owned();
    }
    resolved
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Project")
        .to_owned()
}

fn is_zh(lang: &str) -> bool {
    lang.to_lowercase().starts_with("zh")
}

fn pick_text<'a>(lang: &str, zh: &'a str, en: &'a str) -> &'a str {
    if is_zh(lang) { zh } else { en }
}

fn detect_lang<S: std::hash::BuildHasher>(
    lang: &str,
    nodes: &[Node],
    labels: &HashMap<String, String, S>,
) -> String {
    if !lang.is_empty() && lang.to_lowercase() != "auto" {
        return lang.to_owned();
    }
    let sample: String = labels
        .values()
        .take(50)
        .cloned()
        .chain(nodes.iter().take(200).map(|n| n.label.clone()))
        .chain(nodes.iter().take(100).map(|n| n.source_file.clone()))
        .collect::<Vec<_>>()
        .join(" ");
    // CJK Unified Ideographs
    if sample
        .chars()
        .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    {
        "zh-CN".to_owned()
    } else {
        "en".to_owned()
    }
}

fn truncate_text(text: &str, limit: usize) -> String {
    let s: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.len() <= limit {
        s
    } else {
        format!(
            "{}...",
            &s[..s.len().min(limit.saturating_sub(3))].trim_end()
        )
    }
}

fn humanize_label(label: &str, source_file: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        return Path::new(source_file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_owned();
    }
    if label.starts_with('.') && label.ends_with("()") {
        return label[1..].to_owned();
    }
    let code_exts = [
        ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rs", ".java", ".rb",
    ];
    if code_exts.iter().any(|e| label.ends_with(e)) {
        return Path::new(label)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(label)
            .to_owned();
    }
    if label.contains('_') && !label.contains(' ') && label.len() > 28 {
        let parts: Vec<&str> = label.split('_').filter(|p| !p.is_empty()).collect();
        if !parts.is_empty() {
            let joined = parts[parts.len().saturating_sub(3)..].join(" ");
            return truncate_text(&joined, 42);
        }
    }
    truncate_text(label, 42)
}

fn node_kind(node: &Node) -> &'static str {
    let label = node.label.to_lowercase();
    let source_file = node.source_file.to_lowercase();
    let file_type = node.file_type.to_lowercase();
    let node_type = node.node_type.to_lowercase();
    match node_type.as_str() {
        "class" | "klass" | "struct" | "interface" | "enum" | "trait" | "model" => return "klass",
        "module" | "file" | "package" | "namespace" => return "module",
        "endpoint" | "route" | "api" | "handler" | "controller" => return "api",
        "test" | "spec" => return "test",
        "component" | "hook" | "view" | "page" => return "ui",
        _ => {}
    }
    if file_type == "rationale" || file_type == "document" {
        return "concept";
    }
    if source_file.contains("test") || label.starts_with("test_") || source_file.contains("spec") {
        return "test";
    }
    if ["endpoint", "router", "api", "route"]
        .iter()
        .any(|w| label.contains(w))
    {
        return "api";
    }
    if ["cli", "command", "click", "typer"]
        .iter()
        .any(|w| label.contains(w))
    {
        return "entry";
    }
    if ["async", "await", "stream", "sse"]
        .iter()
        .any(|w| label.contains(w))
    {
        return "async";
    }
    let raw_label = &node.label;
    let hook_like = raw_label.starts_with("use")
        && raw_label.len() > 3
        && raw_label
            .chars()
            .nth(3)
            .is_some_and(|c| c.is_uppercase() || c == '_' || c == '-');
    let sf_lower = source_file.to_lowercase();
    if ["component", "props", "hook", "store"]
        .iter()
        .any(|w| label.contains(w))
        || hook_like
        || matches!(
            std::path::Path::new(&sf_lower)
                .extension()
                .and_then(|e| e.to_str()),
            Some("tsx" | "jsx" | "vue" | "svelte")
        )
    {
        return "ui";
    }
    if raw_label.chars().next().is_some_and(char::is_uppercase) && !raw_label.ends_with("()") {
        return "klass";
    }
    let rl_lower = raw_label.to_lowercase();
    let module_exts = [
        ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rs", ".java", ".kt", ".rb", ".php", ".cs",
        ".swift", ".vue", ".svelte",
    ];
    if module_exts.iter().any(|e| rl_lower.ends_with(e)) {
        return "module";
    }
    "function"
}

fn relation_label(relation: &str, lang: &str) -> String {
    let relation = relation.trim();
    let zh: HashMap<&str, &str> = [
        ("calls", "调用"),
        ("uses", "使用"),
        ("imports", "导入"),
        ("imports_from", "导入"),
        ("method", "方法"),
        ("contains", "包含"),
        ("rationale_for", "说明"),
        ("conceptually_related_to", "相关"),
        ("participate_in", "参与"),
        ("form", "组成"),
    ]
    .iter()
    .copied()
    .collect();
    let en: HashMap<&str, &str> = [
        ("calls", "calls"),
        ("uses", "uses"),
        ("imports", "imports"),
        ("imports_from", "imports"),
        ("method", "method"),
        ("contains", "contains"),
        ("rationale_for", "explains"),
        ("conceptually_related_to", "relates"),
        ("participate_in", "joins"),
        ("form", "forms"),
    ]
    .iter()
    .copied()
    .collect();
    let fallback = relation.replace('_', " ");
    let mapped: &str = if is_zh(lang) {
        zh.get(relation).copied().unwrap_or(fallback.as_str())
    } else {
        en.get(relation).copied().unwrap_or(fallback.as_str())
    };
    safe_mermaid_text(mapped)
}

fn should_include_edge(edge: &CfEdge) -> bool {
    match edge.confidence.as_str() {
        "EXTRACTED" => true,
        "INFERRED" => edge.confidence_score >= 0.85,
        _ => false,
    }
}

fn edge_score(edge: &CfEdge) -> f64 {
    let mut score = edge.confidence_score;
    if edge.confidence == "EXTRACTED" {
        score += 2.0;
    }
    match edge.relation.as_str() {
        "calls" | "uses" | "method" => score += 1.0,
        "imports" | "imports_from" => score += 0.6,
        "contains" => score -= 0.2,
        "rationale_for" => score -= 0.6,
        _ => {}
    }
    score
}

fn preferred_edges(edges: &[CfEdge], allow_structure: bool) -> Vec<&CfEdge> {
    let primary: std::collections::HashSet<&str> =
        ["calls", "uses", "method", "imports", "imports_from"]
            .iter()
            .copied()
            .collect();
    let secondary: std::collections::HashSet<&str> =
        ["contains", "rationale_for", "conceptually_related_to"]
            .iter()
            .copied()
            .collect();
    let mut selected: Vec<&CfEdge> = edges
        .iter()
        .filter(|e| {
            should_include_edge(e)
                && (primary.contains(e.relation.as_str())
                    || (allow_structure && secondary.contains(e.relation.as_str())))
        })
        .collect();
    if selected.is_empty() {
        selected = edges.iter().filter(|e| should_include_edge(e)).collect();
    }
    selected
}

fn mermaid_init(scale: f64, direction: &str) -> String {
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

fn mermaid_class_defs() -> Vec<&'static str> {
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

// ── Community and section indexing ──────────────────────────────────────────

fn build_community_index(nodes: &[Node]) -> IndexMap<String, Vec<usize>> {
    let mut idx: IndexMap<String, Vec<usize>> = IndexMap::new();
    for (i, n) in nodes.iter().enumerate() {
        idx.entry(n.community.clone()).or_default().push(i);
    }
    idx
}

#[allow(clippy::expect_used)] // reason: static literal regex literals cannot fail
fn html_anchor_id(
    raw: &str,
    fallback: &str,
    used: &mut std::collections::HashSet<String>,
) -> String {
    let re = Regex::new(r"[^a-z0-9]+").expect("static regex literal cannot fail");
    let raw_str = if raw.is_empty() { fallback } else { raw };
    let base: String = {
        let lower = raw_str.to_lowercase();
        let slug = re.replace_all(&lower, "-").trim_matches('-').to_owned();
        if slug.is_empty() {
            let fb_lower = fallback.to_lowercase();
            let fb_slug = re.replace_all(&fb_lower, "-").trim_matches('-').to_owned();
            if fb_slug.is_empty() {
                "section".to_owned()
            } else {
                fb_slug
            }
        } else {
            slug
        }
    };
    let base = base[..base.len().min(48)].trim_end_matches('-');
    let base = if base.is_empty() { "section" } else { base };
    let mut candidate = base.to_owned();
    if used.contains(&candidate) {
        let mut hasher = Sha256::new();
        hasher.update(raw_str.as_bytes());
        let hash = hex::encode(&hasher.finalize()[..3]);
        candidate = format!("{base}-{hash}");
    }
    let mut suffix = 2usize;
    while used.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// Normalize a list of sections, ensuring unique IDs and prepending overview.
#[must_use]
pub fn normalize_sections(sections: &[Section], lang: &str) -> Vec<Section> {
    let overview_name = pick_text(lang, "架构总览", "Architecture Overview");
    let mut result = vec![Section {
        id: "overview".to_owned(),
        name: overview_name.to_owned(),
        communities: vec![],
    }];
    let mut used: std::collections::HashSet<String> = ["overview", "hyperedges", "stats"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

    for (index, raw) in sections.iter().enumerate() {
        let raw_id = if raw.id.is_empty() {
            format!("section-{}", index + 1)
        } else {
            raw.id.clone()
        };
        let raw_name = if raw.name.is_empty() {
            raw_id.clone()
        } else {
            raw.name.clone()
        };
        if raw_id.to_lowercase() == "overview" {
            result[0].name = if raw_name.is_empty() {
                overview_name.to_owned()
            } else {
                raw_name
            };
            continue;
        }
        let sid = html_anchor_id(&raw_id, &format!("section-{}", index + 1), &mut used);
        result.push(Section {
            id: sid,
            name: raw_name,
            communities: raw.communities.clone(),
        });
    }
    result
}

/// Architecture keyword archetypes for section classification.
static SECTION_ARCHETYPES: &[(&str, &str, &str, &[&str])] = &[
    (
        "extract-pipeline",
        "提取管线",
        "Extraction Pipeline",
        &[
            "extract",
            "extractor",
            "tree",
            "sitter",
            "parser",
            "language",
            "python",
            "javascript",
            "typescript",
            "rust",
            "java",
            "go",
            "ast",
            "calls",
            "imports",
            "multilang",
        ],
    ),
    (
        "build-graph",
        "图谱构建",
        "Graph Build",
        &[
            "build",
            "graph",
            "merge",
            "dedup",
            "node",
            "edge",
            "hyperedge",
            "json",
            "schema",
            "normalize",
            "confidence",
        ],
    ),
    (
        "analysis-clustering",
        "分析聚类",
        "Analysis & Clustering",
        &[
            "cluster",
            "community",
            "leiden",
            "cohesion",
            "analyze",
            "god",
            "surprise",
            "question",
            "query",
            "path",
            "explain",
            "benchmark",
        ],
    ),
    (
        "outputs-docs",
        "输出文档",
        "Outputs & Docs",
        &[
            "export",
            "html",
            "wiki",
            "obsidian",
            "canvas",
            "svg",
            "graphml",
            "report",
            "callflow",
            "mermaid",
            "tree",
            "documentation",
        ],
    ),
    (
        "cli-skills",
        "CLI 与技能安装",
        "CLI & Skill Installers",
        &[
            "main",
            "install",
            "uninstall",
            "skill",
            "agent",
            "claude",
            "codex",
            "opencode",
            "aider",
            "copilot",
            "kiro",
            "vscode",
            "hook",
            "command",
        ],
    ),
    (
        "ingest-cache-update",
        "摄取与增量更新",
        "Ingestion & Updates",
        &[
            "ingest",
            "fetch",
            "download",
            "url",
            "html",
            "markdown",
            "cache",
            "manifest",
            "watch",
            "update",
            "incremental",
            "transcribe",
            "video",
            "audio",
            "google",
        ],
    ),
    (
        "serve-api",
        "服务 API",
        "Serving API",
        &[
            "serve", "api", "request", "response", "endpoint", "router", "handle", "upload",
            "search", "delete", "enrich",
        ],
    ),
    (
        "security-global",
        "安全与全局图",
        "Security & Global Graph",
        &[
            "security",
            "safe",
            "ssrf",
            "xss",
            "path",
            "traversal",
            "global",
            "prefix",
            "prune",
            "repo",
            "clone",
        ],
    ),
    (
        "tests-fixtures",
        "测试与样例",
        "Tests & Fixtures",
        &[
            "test", "tests", "fixture", "fixtures", "sample", "assert", "pytest", "mock",
        ],
    ),
];

fn community_text(nodes: &[&Node], label: &str) -> String {
    let mut parts = vec![label.to_lowercase()];
    for node in nodes.iter().take(80) {
        parts.push(node.label.to_lowercase());
        parts.push(node.source_file.to_lowercase());
        parts.push(node.node_type.to_lowercase());
        parts.push(node.file_type.to_lowercase());
    }
    parts.join(" ")
}

fn keyword_score(text: &str, keywords: &[&str]) -> usize {
    // The Rust `regex` crate does not support lookbehind, so we cannot directly port
    // Python's `(?<![a-z0-9])kw(?![a-z0-9])`. Instead we split the text into
    // alphanumeric tokens (treating `_`, `-`, `.`, `/` as delimiters, like Python's
    // pattern) and count whole-token matches. This is semantically equivalent for the
    // all-lowercase-ASCII keywords in SECTION_ARCHETYPES.
    let tokens: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let mut score = 0usize;
    for &kw in keywords {
        score += tokens.iter().filter(|&&t| t == kw).count();
    }
    score
}

fn label_for_community<S: std::hash::BuildHasher>(
    cid: &str,
    labels: &HashMap<String, String, S>,
    nodes: &[&Node],
    lang: &str,
) -> String {
    if let Some(l) = labels.get(cid)
        && !l.is_empty()
    {
        return l.clone();
    }
    let kws = section_keywords(nodes, 3);
    if !kws.is_empty() {
        return kws
            .iter()
            .map(|w| {
                let mut c = w.chars();
                c.next().map_or_else(String::new, |f| {
                    f.to_uppercase().collect::<String>() + c.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    pick_text(lang, &format!("社区 {cid}"), &format!("Community {cid}")).to_owned()
}

/// A grouped section used during community classification.
struct GroupedSection {
    id: String,
    name: String,
    communities: Vec<String>,
    node_count: usize,
    priority: usize,
}

/// Derive architecture sections from communities when no sections file is given.
#[must_use]
pub fn derive_sections_from_communities<S: std::hash::BuildHasher>(
    nodes: &[Node],
    labels: &HashMap<String, String, S>,
    lang: &str,
    max_sections: usize,
) -> Vec<Section> {
    let comm_idx = build_community_index(nodes);
    let mut sections = vec![Section {
        id: "overview".to_owned(),
        name: pick_text(lang, "架构总览", "Architecture Overview").to_owned(),
        communities: vec![],
    }];

    let mut grouped: IndexMap<String, GroupedSection> = IndexMap::new();
    let mut unassigned: Vec<(String, Vec<usize>, String)> = vec![];

    // Sort communities largest-first.
    let mut sorted_comms: Vec<(&String, &Vec<usize>)> = comm_idx.iter().collect();
    sorted_comms.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));

    for (cid, node_indices) in &sorted_comms {
        let comm_nodes: Vec<&Node> = node_indices.iter().map(|&i| &nodes[i]).collect();
        let label = label_for_community(cid, labels, &comm_nodes, lang);
        let text = community_text(&comm_nodes, &label);

        // Find the best matching archetype.
        let mut best_sid: Option<(&str, &str, &str, usize)> = None;
        let mut best_score = 0usize;
        for (priority, (sid, zh, en, keywords)) in SECTION_ARCHETYPES.iter().enumerate() {
            let score = keyword_score(&text, keywords);
            if score > best_score {
                best_score = score;
                best_sid = Some((sid, zh, en, priority));
            }
        }

        if let Some((sid, zh_name, en_name, priority)) = best_sid.filter(|_| best_score >= 2) {
            let sec = grouped
                .entry((*sid).to_owned())
                .or_insert_with(|| GroupedSection {
                    id: (*sid).to_owned(),
                    name: pick_text(lang, zh_name, en_name).to_owned(),
                    communities: vec![],
                    node_count: 0,
                    priority,
                });
            sec.communities.push((*cid).clone());
            sec.node_count += node_indices.len();
        } else {
            unassigned.push(((*cid).clone(), (*node_indices).clone(), label));
        }
    }

    // Rank grouped sections.
    let mut ranked: Vec<GroupedSection> = grouped.into_values().collect();
    ranked.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(b.node_count.cmp(&a.node_count))
            .then(a.id.cmp(&b.id))
    });

    let cap = max_sections.max(1).saturating_sub(1);
    let selected: Vec<GroupedSection> = ranked.drain(..ranked.len().min(cap)).collect();
    let overflow: Vec<GroupedSection> = ranked;

    sections.extend(selected.into_iter().map(|s| Section {
        id: s.id,
        name: s.name,
        communities: s.communities,
    }));

    let mut overflow_communities: Vec<String> = vec![];
    for s in overflow {
        overflow_communities.extend(s.communities);
    }

    let remaining_slots = max_sections
        .saturating_sub(sections.len().saturating_sub(1))
        .saturating_sub(1);
    for (cid, _, label) in unassigned.iter().take(remaining_slots) {
        sections.push(Section {
            id: if label.is_empty() {
                format!("community-{cid}")
            } else {
                label.clone()
            },
            name: label.clone(),
            communities: vec![cid.clone()],
        });
    }
    overflow_communities.extend(
        unassigned[remaining_slots.min(unassigned.len())..]
            .iter()
            .map(|(cid, _, _)| cid.clone()),
    );
    if !overflow_communities.is_empty() {
        sections.push(Section {
            id: "other".to_owned(),
            name: pick_text(lang, "其他", "Other").to_owned(),
            communities: overflow_communities,
        });
    }
    sections
}

fn build_section_node_map(
    sections: &[Section],
    comm_idx: &IndexMap<String, Vec<usize>>,
) -> IndexMap<String, Vec<usize>> {
    let mut map: IndexMap<String, Vec<usize>> = IndexMap::new();
    for sec in sections {
        let sid = &sec.id;
        if sid == "overview" {
            map.insert(sid.clone(), vec![]);
            continue;
        }
        let mut idxs = vec![];
        for cid in &sec.communities {
            if let Some(v) = comm_idx.get(cid.as_str()) {
                idxs.extend_from_slice(v);
            }
        }
        map.insert(sid.clone(), idxs);
    }
    map
}

// ── Edge classification ──────────────────────────────────────────────────────

pub(crate) struct ClassifiedEdges {
    pub(crate) intra: IndexMap<String, Vec<usize>>, // section_id -> edge indices
    pub(crate) inter: Vec<usize>,
    node_section: HashMap<String, String>,
}

fn classify_edges(
    edges: &[CfEdge],
    section_nodes_map: &IndexMap<String, Vec<usize>>,
    nodes: &[Node],
) -> ClassifiedEdges {
    let mut node_section: HashMap<String, String> = HashMap::new();
    for (sid, idxs) in section_nodes_map {
        for &i in idxs {
            node_section.insert(nodes[i].id.clone(), sid.clone());
        }
    }
    let mut intra: IndexMap<String, Vec<usize>> = IndexMap::new();
    let mut inter: Vec<usize> = vec![];

    for (ei, e) in edges.iter().enumerate() {
        let src_sec = node_section.get(&e.source);
        let tgt_sec = node_section.get(&e.target);
        match (src_sec, tgt_sec) {
            (None, _) | (_, None) => {} // orphan — not tracked
            (Some(ss), Some(ts)) if ss == ts => intra.entry(ss.clone()).or_default().push(ei),
            (Some(_ss), Some(_ts)) => inter.push(ei),
        }
    }
    ClassifiedEdges {
        intra,
        inter,
        node_section,
    }
}

fn section_edge_summary(
    classified: &ClassifiedEdges,
    edges: &[CfEdge],
) -> IndexMap<(String, String), (usize, String)> {
    let mut summary: IndexMap<(String, String), (usize, IndexMap<String, usize>)> = IndexMap::new();
    for &ei in &classified.inter {
        let e = &edges[ei];
        if !should_include_edge(e) {
            continue;
        }
        let src_sec = classified.node_section.get(&e.source);
        let tgt_sec = classified.node_section.get(&e.target);
        match (src_sec, tgt_sec) {
            (Some(ss), Some(ts)) if ss != ts => {
                let entry = summary
                    .entry((ss.clone(), ts.clone()))
                    .or_insert((0, IndexMap::new()));
                entry.0 += 1;
                *entry.1.entry(e.relation.clone()).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    // Convert to (count, most-common-relation).
    summary
        .into_iter()
        .map(|(k, (count, rels))| {
            let top_rel = rels
                .iter()
                .max_by_key(|(_, c)| *c)
                .map_or("relates", |(r, _)| r.as_str())
                .to_owned();
            (k, (count, top_rel))
        })
        .collect()
}

// ── Mermaid diagram generators ───────────────────────────────────────────────

fn node_degree_scores<'a>(edges: &'a [&'a CfEdge]) -> HashMap<&'a str, f64> {
    let mut scores: HashMap<&'a str, f64> = HashMap::new();
    for e in edges {
        let s = edge_score(e);
        *scores.entry(e.source.as_str()).or_insert(0.0) += s;
        *scores.entry(e.target.as_str()).or_insert(0.0) += s;
    }
    scores
}

fn select_diagram_nodes<'a>(
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

/// Generate the architecture overview Mermaid diagram.
#[must_use]
pub(crate) fn generate_overview_graph(
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

/// Generate a compact call-flow chart for a single section.
/// Parameters for [`generate_section_flowchart`].
pub(crate) struct FlowchartParams<'a> {
    pub(crate) section_id: &'a str,
    pub(crate) section_name: &'a str,
    pub(crate) nodes: &'a [Node],
    pub(crate) edges: &'a [CfEdge],
    pub(crate) lang: &'a str,
    pub(crate) diagram_scale: f64,
    pub(crate) max_nodes: usize,
    pub(crate) max_edges: usize,
}

#[must_use]
pub(crate) fn generate_section_flowchart(p: &FlowchartParams<'_>) -> String {
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

// ── HTML generators ──────────────────────────────────────────────────────────

fn generate_nav(sections: &[Section]) -> String {
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

fn node_display_name(node: Option<&Node>, fallback: &str) -> String {
    match node {
        None => fallback.to_owned(),
        Some(n) => {
            let label = if n.label.is_empty() {
                fallback.to_owned()
            } else {
                n.label.clone()
            };
            humanize_label(&label, &n.source_file)
        }
    }
}

fn format_node_refs(
    node_ids: &[&str],
    nodes: &[Node],
    lang: &str,
    empty_text: &str,
    limit: usize,
) -> String {
    if node_ids.is_empty() {
        return htmlescape::encode_minimal(empty_text);
    }
    let node_by_id: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut sorted: Vec<&str> = node_ids.to_vec();
    sorted.sort_by_key(|&nid| node_display_name(node_by_id.get(nid).copied(), nid).to_lowercase());
    let mut parts: Vec<String> = sorted
        .iter()
        .take(limit)
        .map(|&nid| {
            let node = node_by_id.get(nid).copied();
            let label = node_display_name(node, nid);
            let source = node
                .map(|n| safe_file_path(&n.source_file))
                .unwrap_or_default();
            if source.is_empty() {
                format!("<code>{}</code>", htmlescape::encode_minimal(&label))
            } else {
                format!(
                    "<code>{}</code><br><small style=\"color:var(--muted)\">{}</small>",
                    htmlescape::encode_minimal(&label),
                    htmlescape::encode_minimal(&source)
                )
            }
        })
        .collect();
    if node_ids.len() > limit {
        let more = node_ids.len() - limit;
        parts.push(htmlescape::encode_minimal(pick_text(
            lang,
            &format!("+{more} 个更多"),
            &format!("+{more} more"),
        )));
    }
    parts.join("<br>")
}

fn suggest_tag(label: &str, file_type: &str, lang: &str, kind: &str) -> String {
    let names: &[(&str, &str, &str, &str)] = &[
        ("concept", "概念", "Concept", "tag-func"),
        ("entry", "入口", "Entry", "tag-cmd"),
        ("api", "API", "API", "tag-endpoint"),
        ("async", "异步", "Async", "tag-async"),
        ("klass", "类", "Class", "tag-class"),
        ("ui", "UI", "UI", "tag-hook"),
        ("module", "模块", "Module", "tag-class"),
        ("test", "测试", "Test", "tag-func"),
        ("function", "函数", "Function", "tag-func"),
    ];
    for &(k, zh, en, cls) in names {
        if kind == k {
            let text = pick_text(lang, zh, en);
            return format!("<span class=\"tag {cls}\">{text}</span>");
        }
    }
    if file_type == "rationale" {
        return format!(
            "<span class=\"tag tag-func\">{}</span>",
            pick_text(lang, "概念", "Concept")
        );
    }
    let lower = label.to_lowercase();
    if lower.contains("router") || lower.contains("endpoint") || lower.contains("/api/") {
        return format!(
            "<span class=\"tag tag-endpoint\">{}</span>",
            pick_text(lang, "API端点", "API")
        );
    }
    if lower.contains("async") || lower.contains("await") || lower.contains("stream") {
        return format!(
            "<span class=\"tag tag-async\">{}</span>",
            pick_text(lang, "异步", "Async")
        );
    }
    if lower.contains("class") || lower.contains("model") || lower.contains("schema") {
        return format!(
            "<span class=\"tag tag-class\">{}</span>",
            pick_text(lang, "类", "Class")
        );
    }
    if lower.contains("hook") || lower.contains("usestate") || lower.contains("useeffect") {
        return "<span class=\"tag tag-hook\">Hook</span>".to_owned();
    }
    if lower.contains("component") || lower.contains("props") {
        return format!(
            "<span class=\"tag tag-class\">{}</span>",
            pick_text(lang, "组件", "Component")
        );
    }
    format!(
        "<span class=\"tag tag-func\">{}</span>",
        pick_text(lang, "函数", "Function")
    )
}

#[allow(clippy::too_many_lines)] // Long dispatch function; splitting into sub-functions would obscure the linear logic.
fn describe_node(label: &str, source_file: &str, file_type: &str, lang: &str) -> String {
    let lower = label.to_lowercase();
    let source = if source_file.is_empty() {
        pick_text(lang, "项目", "project")
    } else {
        source_file
    };
    if file_type == "rationale" {
        return pick_text(
            lang,
            &format!("设计说明：{label}"),
            &format!("Design note for {label}."),
        )
        .to_owned();
    }
    if file_type == "document" {
        return pick_text(
            lang,
            &format!("文档入口，描述 {label} 相关能力。"),
            &format!("Documentation node describing {label}."),
        )
        .to_owned();
    }
    if matches!(
        std::path::Path::new(label)
            .extension()
            .and_then(|e| e.to_str()),
        Some("py" | "tsx" | "ts")
    ) {
        return pick_text(
            lang,
            &format!("{source} 中的模块文件，承载该层主要实现。"),
            &format!("Module file in {source}."),
        )
        .to_owned();
    }
    if lower.contains("config") {
        return pick_text(
            lang,
            "读取、解析或持久化项目配置。",
            "Reads, resolves, or persists project configuration.",
        )
        .to_owned();
    }
    if lower.contains("scan") {
        return pick_text(
            lang,
            "触发项目扫描或处理扫描状态。",
            "Starts scanning or handles scan status.",
        )
        .to_owned();
    }
    if lower.contains("ingest") || lower.contains("clone") || lower.contains("git") {
        return pick_text(
            lang,
            "把本地目录或远程仓库转换为分析上下文。",
            "Turns a local path or remote repository into analysis context.",
        )
        .to_owned();
    }
    if lower.contains("prompt") {
        return pick_text(
            lang,
            "构造发送给 LLM 的结构化提示。",
            "Builds structured prompts for model calls.",
        )
        .to_owned();
    }
    if lower.contains("analy") {
        return pick_text(
            lang,
            "编排分析流程并产出结构化文档数据。",
            "Orchestrates analysis and returns structured documentation data.",
        )
        .to_owned();
    }
    if lower.contains("graph") || lower.contains("dependency") {
        return pick_text(
            lang,
            "构建依赖关系并提供排序或图形化数据。",
            "Builds dependency relationships and graph data.",
        )
        .to_owned();
    }
    if lower.contains("export") || lower.contains("markdown") || lower.contains("html") {
        return pick_text(
            lang,
            "将文档数据导出为目标格式。",
            "Exports documentation data to a target format.",
        )
        .to_owned();
    }
    if lower.contains("chat") || lower.contains("rag") || lower.contains("retrieve") {
        return pick_text(
            lang,
            "支撑检索增强问答或流式聊天。",
            "Supports retrieval-augmented Q&A or streaming chat.",
        )
        .to_owned();
    }
    if lower.contains("wiki") || lower.contains("page") || lower.contains("sidebar") {
        return pick_text(
            lang,
            "组织文档页面、侧边栏或内容读取。",
            "Organizes documentation pages, navigation, or content lookup.",
        )
        .to_owned();
    }
    if lower.contains("cache") || lower.contains("hash") {
        return pick_text(
            lang,
            "缓存分析结果或生成缓存键。",
            "Caches analysis results or computes cache keys.",
        )
        .to_owned();
    }
    if lower.contains("test") {
        return pick_text(
            lang,
            "验证导入、入口点或版本等基础行为。",
            "Verifies imports, entry points, or version behavior.",
        )
        .to_owned();
    }
    pick_text(
        lang,
        &format!("{source} 中的 {label} 节点。"),
        &format!("{label} node in {source}."),
    )
    .to_owned()
}

fn generate_call_table_rows(nodes: &[Node], section_edges: &[CfEdge], lang: &str) -> String {
    if nodes.is_empty() {
        return String::new();
    }
    let node_by_id: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
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

        // Gather all nodes for reference lookup.
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
        let _ = node_by_id.get(nid); // suppress potential dead_code note

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

fn generate_header(
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

fn generate_overview_cards(
    meta: &IndexMap<String, serde_json::Value>,
    report_text: &str,
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
    let _ = (meta, report_text); // used by caller
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

fn section_keywords(nodes: &[&Node], limit: usize) -> Vec<String> {
    let stopwords: std::collections::HashSet<&str> = [
        "the", "and", "for", "with", "from", "this", "that", "class", "function", "method", "file",
        "src", "lib", "core", "index", "main", "init", "py", "ts", "tsx", "js", "jsx", "go", "rs",
        "java", "html", "css",
    ]
    .iter()
    .copied()
    .collect();
    // Use IndexMap to preserve insertion order — matches Python Counter.most_common()
    // behaviour where ties are broken by insertion (first-seen) order.
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for node in nodes {
        let text = format!("{} {}", node.label, node.source_file)
            .replace('/', " ")
            .replace(['_', '-'], " ");
        for raw in text.split_whitespace() {
            let word: String = raw
                .chars()
                .filter(|c| c.is_alphanumeric())
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .collect();
            if word.len() >= 3 && !stopwords.contains(word.as_str()) {
                *counts.entry(word).or_insert(0) += 1;
            }
        }
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    // Stable sort by count-descending preserves insertion order for ties,
    // matching Python's Counter.most_common() behaviour.
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    sorted.into_iter().take(limit).map(|(w, _)| w).collect()
}

fn generate_section_intro(sec: &Section, nodes: &[Node], edge_count: usize, lang: &str) -> String {
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

fn generate_section_cards(
    sec: &Section,
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
    let _ = sec;
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

#[allow(clippy::expect_used)] // reason: static literal regex cannot fail
fn report_highlights(report_text: &str, lang: &str) -> String {
    if report_text.trim().is_empty() {
        return String::new();
    }

    let re_numbered = Regex::new(r"^\d+\.").expect("static regex literal cannot fail");
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

// ── Resolve paths ─────────────────────────────────────────────────────────────

struct ResolvedPaths {
    base: PathBuf,
    graphify_out: PathBuf,
    graph: PathBuf,
    report: PathBuf,
    labels: PathBuf,
    sections: Option<PathBuf>,
}

fn resolve_graphify_paths(opts: &CallflowOptions) -> ResolvedPaths {
    let base = opts
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let graphify_out = if let Some(ref p) = opts.graphify_out {
        p.clone()
    } else if let Some(ref g) = opts.graph {
        g.parent().map_or_else(|| base.clone(), Path::to_path_buf)
    } else if base.join("graph.json").exists() {
        base.clone()
    } else {
        base.join("graphify-out")
    };

    let project_root = if graphify_out.file_name().and_then(|n| n.to_str()) == Some("graphify-out")
    {
        graphify_out
            .parent()
            .map_or_else(|| base.clone(), Path::to_path_buf)
    } else {
        base.clone()
    };

    let graph = opts
        .graph
        .clone()
        .unwrap_or_else(|| graphify_out.join("graph.json"));
    let report = opts
        .report
        .clone()
        .unwrap_or_else(|| graphify_out.join("GRAPH_REPORT.md"));
    let labels = opts
        .labels
        .clone()
        .unwrap_or_else(|| graphify_out.join(".graphify_labels.json"));
    let sections = opts.sections.clone();

    ResolvedPaths {
        base: project_root,
        graphify_out,
        graph,
        report,
        labels,
        sections,
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

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
    let paths = resolve_graphify_paths(opts);

    if !paths.graph.exists() {
        return Err(HtmlError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "graphify output not found: {}. Run graphify first or pass --graph /path/to/graph.json.",
                paths.graph.display()
            ),
        )));
    }

    let (nodes, edges, hyperedges, mut meta) = load_graph(&paths.graph)?;
    let labels = load_labels(Some(&paths.labels));
    let lang = detect_lang(&opts.lang, &nodes, &labels);

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
        derive_sections_from_communities(&nodes, &labels, &lang, opts.max_sections)
    };

    let sections = normalize_sections(&sections, &lang);
    let report_text = load_report(Some(&paths.report));

    if nodes.is_empty() {
        return Err(HtmlError::EmptyGraph);
    }
    if sections.len() <= 1 {
        return Err(HtmlError::NoSections);
    }

    meta.insert(
        "project_name".to_owned(),
        serde_json::Value::String(infer_project_name(&paths.graph, &meta)),
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
        paths
            .graphify_out
            .join(format!("{}-callflow.html", safe_filename(project_name)))
    };

    let comm_idx = build_community_index(&nodes);
    meta.insert(
        "community_count".to_owned(),
        serde_json::Value::Number(comm_idx.len().into()),
    );

    let section_nodes_map = build_section_node_map(&sections, &comm_idx);
    let classified = classify_edges(&edges, &section_nodes_map, &nodes);

    let project_name = meta
        .get("project_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Project");
    let lang_str = lang.as_str();
    let doc_title = if is_zh(lang_str) {
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
        CSS,
    );

    html.push_str(&generate_header(&sections, &meta, lang_str));

    // Architecture Overview.
    let overview_name = sections
        .first()
        .map_or("Architecture Overview", |s| s.name.as_str());
    let _ = write!(
        html,
        "<!-- ====== Architecture Overview ====== -->\n<h2 id=\"overview\">1. {}</h2>\n\n<div class=\"mermaid\">\n",
        htmlescape::encode_minimal(overview_name)
    );
    html.push_str(&generate_overview_graph(
        &sections,
        &section_nodes_map,
        &classified,
        &edges,
        lang_str,
        opts.diagram_scale,
    ));
    html.push_str("\n</div>\n");
    html.push_str(&generate_overview_cards(
        &meta,
        &report_text,
        &sections,
        &section_nodes_map,
        &classified,
        &edges,
        lang_str,
    ));
    let report_card = report_highlights(&report_text, lang_str);
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
        let name = &sec.name;

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
        let edge_count = sec_edges.len();

        let h3_title = pick_text(lang_str, "调用明细", "Call Details");
        let number_header = "#";
        let function_header = pick_text(lang_str, "节点", "Node");
        let type_header = pick_text(lang_str, "类型", "Type");
        let inbound_header = pick_text(lang_str, "调用方", "Caller");
        let outbound_header = pick_text(lang_str, "被调用/依赖", "Callees");
        let desc_header = pick_text(lang_str, "说明", "Description");

        let _ = write!(
            html,
            "<!-- ====== {section_num}. {} ====== -->\n<h2 id=\"{}\">{section_num}. {}</h2>\n{}\n\n<div class=\"mermaid\">\n{}\n</div>\n\n<h3>{h3_title}</h3>\n<table class=\"call-table\">\n<tr>\n  <th style=\"width:5%\">{number_header}</th>\n  <th style=\"width:28%\">{function_header}</th>\n  <th style=\"width:10%\">{type_header}</th>\n  <th style=\"width:17%\">{inbound_header}</th>\n  <th style=\"width:20%\">{outbound_header}</th>\n  <th style=\"width:20%\">{desc_header}</th>\n</tr>\n{}</table>\n\n{}\n<hr>\n",
            html_comment_text(name),
            htmlescape::encode_attribute(sid),
            htmlescape::encode_minimal(name),
            generate_section_intro(sec, &sec_nodes, edge_count, lang_str),
            generate_section_flowchart(&FlowchartParams {
                section_id: sid,
                section_name: name,
                nodes: &sec_nodes,
                edges: &sec_edges,
                lang: lang_str,
                diagram_scale: opts.diagram_scale,
                max_nodes: opts.max_diagram_nodes,
                max_edges: opts.max_diagram_edges,
            }),
            generate_call_table_rows(&sec_nodes, &sec_edges, lang_str),
            generate_section_cards(sec, &sec_nodes, &sec_edges, lang_str),
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
    let now = Utc::now().format("%Y-%m-%d %H:%M UTC");
    let _ = write!(
        html,
        "<div style=\"text-align:center; padding:40px 0; color: var(--muted); font-size:0.9rem;\">\n  <p>{} — Architecture Documentation</p>\n  <p>Generated: {} · graphify callflow-html</p>\n</div>\n",
        htmlescape::encode_minimal(project_name),
        now,
    );

    // Close.
    html.push_str("</div><!-- .container -->\n\n");
    html.push_str(JS_FOOTER);
    html.push_str("\n\n</body>\n</html>");

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, html.as_bytes())?;
    Ok(output_path)
}
