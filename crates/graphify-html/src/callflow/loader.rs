//! Data loading, normalization, and text-manipulation helpers.
//!
//! Extracted from the original monolithic `callflow.rs` so that the graph
//! ingestion logic (`load_graph`, `load_labels`, `load_report`) and the
//! string / Mermaid-id helpers can be read and tested in isolation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::HtmlError;

use super::options::{CfEdge, Node};

// ── Low-level JSON helpers ──────────────────────────────────────────────────

/// Return the first non-empty string value found under any of `keys` in `map`.
pub(super) fn first_str_val<'a>(
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

/// Coerce a JSON value to `f64`, falling back to `default` on non-numeric input.
pub(super) fn to_float(v: &serde_json::Value, default: f64) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(default),
        serde_json::Value::String(s) => s.parse().unwrap_or(default),
        _ => default,
    }
}

// ── Node / edge normalization ───────────────────────────────────────────────

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

/// Extract a node-id string from an edge endpoint field, handling both string
/// and nested-object shapes.
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

// ── Graph loading ───────────────────────────────────────────────────────────

/// Parsed contents of a `graph.json` file: `(nodes, edges, hyperedges, meta)`.
pub(super) type GraphData = (
    Vec<Node>,
    Vec<CfEdge>,
    Vec<serde_json::Value>,
    IndexMap<String, serde_json::Value>,
);

/// Load graph.json. Returns `(nodes, edges, hyperedges, meta)`.
///
/// # Errors
/// Returns [`HtmlError::Io`] on file read error or malformed JSON (parse
/// failures are wrapped into [`std::io::ErrorKind::InvalidData`] via
/// `HtmlError::Io`, not into a separate parse variant), or
/// [`HtmlError::Security`] if the file exceeds the memory-bomb size cap.
pub fn load_graph(path: &Path) -> Result<GraphData, HtmlError> {
    graphify_security::check_graph_file_size_cap(path)?;
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

// ── Mermaid-safe label / ID helpers ────────────────────────────────────────

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

/// Build a Mermaid-safe ASCII identifier with a SHA-256 (truncated) hash suffix.
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

// ── Language detection ──────────────────────────────────────────────────────

/// Return `true` if the language tag starts with `"zh"` (Chinese).
pub(super) fn is_zh(lang: &str) -> bool {
    lang.to_lowercase().starts_with("zh")
}

/// Select `zh` or `en` based on the language tag.
pub(super) fn pick_text<'a>(lang: &str, zh: &'a str, en: &'a str) -> &'a str {
    if is_zh(lang) { zh } else { en }
}

/// Auto-detect language from node labels and community labels when `lang` is `"auto"`.
///
/// Scans up to 50 label values and 200 node labels for CJK characters; returns
/// `"zh-CN"` if found, otherwise `"en"`. When `lang` is already set explicitly,
/// it is returned unchanged.
pub(super) fn detect_lang<S: std::hash::BuildHasher>(
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

// ── Path resolution ─────────────────────────────────────────────────────────

/// Resolved filesystem paths used throughout the callflow rendering pipeline.
pub(super) struct ResolvedPaths {
    /// Project root directory (parent of `graphify-out/` when present).
    pub(super) base: PathBuf,
    /// Directory containing `graph.json` and related output files.
    pub(super) graphify_out: PathBuf,
    /// Path to the `graph.json` input file.
    pub(super) graph: PathBuf,
    /// Path to the optional `GRAPH_REPORT.md` file.
    pub(super) report: PathBuf,
    /// Path to the optional `.graphify_labels.json` community-labels file.
    pub(super) labels: PathBuf,
    /// Path to the optional sections JSON file; `None` when not provided.
    pub(super) sections: Option<PathBuf>,
}

/// Resolve all file paths needed by the callflow renderer from the options struct.
///
/// Handles the heuristic of locating `graphify-out/` when neither `--graph` nor
/// `--graphify-out` is explicitly provided. Returns a `ResolvedPaths` bundle
/// used throughout the callflow pipeline.
pub(super) fn resolve_graphify_paths(opts: &super::options::CallflowOptions) -> ResolvedPaths {
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
