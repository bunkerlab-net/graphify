//! `GRAPH_REPORT.md` renderer. Ports `graphify-py/graphify/report.py`.
//!
//! Public entry points are [`render_report`] and [`write_report`].
//!
//! # Analysis value shape
//!
//! `analysis` must be a JSON object with the following fields (all optional
//! fields default gracefully):
//!
//! ```json
//! {
//!   "communities":           { "<cid>": ["node_id", ...], ... },
//!   "cohesion_scores":       { "<cid>": 0.75, ... },
//!   "community_labels":      { "<cid>": "Label", ... },
//!   "god_nodes":             [{ "id": "...", "label": "...", "degree": 5 }, ...],
//!   "surprising_connections":[{ "source": "...", "target": "...", ... }, ...],
//!   "detection_result":      { "total_files": 4, "total_words": 62400, "warning": null },
//!   "token_cost":            { "input": 1200, "output": 340 },
//!   "root":                  "./project",
//!   "suggested_questions":   null,
//!   "min_community_size":    3,
//!   "built_at_commit":       null
//! }
//! ```

mod sections;

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use chrono::Local;
use graphify_build::Graph;
use regex::Regex;
use serde_json::Value;
use thiserror::Error;

use sections::{
    communities::{render_communities, render_nav_hubs},
    detection::{
        ConfidenceStats, render_ambiguous, render_corpus_check, render_gaps, render_summary,
    },
    god_nodes::render_god_nodes,
    header::render_freshness,
    suggestions::render_questions,
    surprises::{render_hyperedges, render_surprising},
};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from [`write_report`].
#[derive(Debug, Error)]
pub enum ReportError {
    /// I/O error writing the report file.
    #[error("graphify: failed to write report: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// `safe_community_name` — mirrors Python's `_safe_community_name`
// ---------------------------------------------------------------------------

/// Characters to strip from community labels when building Obsidian wikilinks.
///
/// Regex literal is validated at compile-time via the `LazyLock` initialiser.
static UNSAFE_CHARS: LazyLock<Regex> = LazyLock::new(|| {
    #[allow(clippy::unwrap_used)] // reason: literal pattern is known-good at compile time
    Regex::new(r#"[\\/*?:"<>|#^\[\]]"#).unwrap()
});

/// Strip `.md` / `.mdx` / `.markdown` suffix (case-insensitive).
static MD_EXT: LazyLock<Regex> = LazyLock::new(|| {
    #[allow(clippy::unwrap_used)] // reason: literal pattern is known-good at compile time
    Regex::new(r"(?i)\.(md|mdx|markdown)$").unwrap()
});

/// Mirrors Python `_safe_community_name`.
pub(crate) fn safe_community_name(label: &str) -> String {
    let normalised = label.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let cleaned = UNSAFE_CHARS.replace_all(&normalised, "");
    let cleaned = cleaned.trim();
    let cleaned = MD_EXT.replace(cleaned, "");
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned.into_owned()
    }
}

// ---------------------------------------------------------------------------
// Analysis extraction helpers
// ---------------------------------------------------------------------------

type Communities<'a> = Vec<(i64, Vec<&'a str>)>;

fn extract_communities(obj: &serde_json::Map<String, Value>) -> Communities<'_> {
    obj.get("communities")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    let cid = k.parse::<i64>().ok()?;
                    let nodes = v.as_array()?.iter().filter_map(Value::as_str).collect();
                    Some((cid, nodes))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_cohesion(obj: &serde_json::Map<String, Value>) -> HashMap<i64, f64> {
    obj.get("cohesion_scores")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.parse::<i64>().ok()?, v.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_labels(obj: &serde_json::Map<String, Value>) -> HashMap<i64, &str> {
    obj.get("community_labels")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.parse::<i64>().ok()?, v.as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Core renderer
// ---------------------------------------------------------------------------

/// Render a `GRAPH_REPORT.md` string from a graph and analysis result.
///
/// The output is byte-identical to the Python `generate()` function for the
/// same inputs.
#[must_use]
#[allow(clippy::too_many_lines)] // reason: mirrors the Python generate() function which is a single sequential renderer
pub fn render_report(graph: &Graph, analysis: &Value) -> String {
    let today = Local::now().format("%Y-%m-%d").to_string();

    let empty_obj = serde_json::Map::new();
    let obj = analysis.as_object().unwrap_or(&empty_obj);

    let root = obj.get("root").and_then(Value::as_str).unwrap_or_default();

    let min_community_size = usize::try_from(
        obj.get("min_community_size")
            .and_then(Value::as_u64)
            .unwrap_or(3),
    )
    .unwrap_or(3);

    let built_at_commit = obj
        .get("built_at_commit")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());

    let communities: Communities<'_> = extract_communities(obj);
    let cohesion_scores = extract_cohesion(obj);
    let community_labels = extract_labels(obj);

    let empty_arr = Vec::new();
    let god_node_list = obj
        .get("god_nodes")
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);
    let surprise_list = obj
        .get("surprising_connections")
        .and_then(Value::as_array)
        .unwrap_or(&empty_arr);
    let detection = obj.get("detection_result").and_then(Value::as_object);
    let token_cost = obj.get("token_cost").and_then(Value::as_object);
    let suggested_questions = obj.get("suggested_questions").and_then(Value::as_array);

    // Edge confidence stats
    let confidences: Vec<&str> = graph
        .edges()
        .map(|e| {
            e.attrs
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("EXTRACTED")
        })
        .collect();
    let total = confidences.len().max(1);
    let ext_pct =
        u64::try_from(confidences.iter().filter(|&&c| c == "EXTRACTED").count() * 100 / total)
            .unwrap_or(0);
    let inf_pct =
        u64::try_from(confidences.iter().filter(|&&c| c == "INFERRED").count() * 100 / total)
            .unwrap_or(0);
    let amb_pct =
        u64::try_from(confidences.iter().filter(|&&c| c == "AMBIGUOUS").count() * 100 / total)
            .unwrap_or(0);

    let inf_edges: Vec<&graphify_build::Edge> = graph
        .edges()
        .filter(|e| e.attrs.get("confidence").and_then(Value::as_str) == Some("INFERRED"))
        .collect();
    let inf_scores: Vec<f64> = inf_edges
        .iter()
        .map(|e| {
            e.attrs
                .get("confidence_score")
                .and_then(Value::as_f64)
                .unwrap_or(0.5)
        })
        .collect();
    let inf_avg = if inf_scores.is_empty() {
        None
    } else {
        let sum: f64 = inf_scores.iter().sum();
        #[allow(clippy::cast_precision_loss)] // reason: small list, precision loss negligible
        Some((sum / inf_scores.len() as f64 * 100.0).round() / 100.0)
    };

    // Non-empty communities (have at least one non-file node)
    let non_empty: Vec<(i64, &Vec<&str>)> = communities
        .iter()
        .filter(|(_, nodes)| nodes.iter().any(|n| !sections::is_file_node(graph, n)))
        .map(|(cid, nodes)| (*cid, nodes))
        .collect();

    let thin_count_summary = communities
        .iter()
        .filter(|(_, nodes)| {
            let real = nodes
                .iter()
                .filter(|n| !sections::is_file_node(graph, n))
                .count();
            real > 0 && real < min_community_size
        })
        .count();
    let shown_count = communities.len() - thin_count_summary;

    // Isolated nodes (for the gaps section)
    let isolated: Vec<&str> = graph
        .nodes()
        .filter(|(id, attrs)| {
            sections::node_degree(graph, id) <= 1
                && !sections::is_file_node(graph, id)
                && !sections::is_concept_node(graph, id)
                && attrs.get("file_type").and_then(Value::as_str) != Some("rationale")
        })
        .map(|(id, _)| id.as_str())
        .collect();

    let thin_community_count = communities
        .iter()
        .filter(|(_, nodes)| {
            let real = nodes
                .iter()
                .filter(|n| !sections::is_file_node(graph, n))
                .count();
            real > 0 && real < 3
        })
        .count();

    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("# Graph Report - {root}  ({today})"));
    lines.push(String::new());
    render_corpus_check(&mut lines, detection);
    let stats = ConfidenceStats {
        ext_pct,
        inf_pct,
        amb_pct,
        inf_edges_len: inf_edges.len(),
        inf_avg,
    };
    render_summary(
        &mut lines,
        graph,
        communities.len(),
        thin_count_summary,
        shown_count,
        &stats,
        token_cost,
    );
    if let Some(commit) = built_at_commit {
        render_freshness(&mut lines, commit);
    }
    if !non_empty.is_empty() {
        render_nav_hubs(&mut lines, &non_empty, &community_labels);
    }
    render_god_nodes(&mut lines, god_node_list);
    render_surprising(&mut lines, surprise_list);

    // Hyperedges (only if present and non-empty)
    if let Some(hyperedges) = graph
        .graph_attrs
        .get("hyperedges")
        .and_then(Value::as_array)
        && !hyperedges.is_empty()
    {
        render_hyperedges(&mut lines, hyperedges);
    }

    render_communities(
        &mut lines,
        graph,
        &communities,
        &cohesion_scores,
        &community_labels,
        thin_count_summary,
        min_community_size,
    );
    render_ambiguous(&mut lines, graph);
    render_gaps(
        &mut lines,
        graph,
        thin_community_count,
        &isolated,
        min_community_size,
        amb_pct,
    );

    if let Some(qs) = suggested_questions {
        render_questions(&mut lines, qs);
    }

    lines.join("\n")
}

/// Write a `GRAPH_REPORT.md` to `path`.
///
/// # Errors
///
/// Returns [`ReportError::Io`] if the file cannot be written.
pub fn write_report(graph: &Graph, analysis: &Value, path: &Path) -> Result<(), ReportError> {
    let content = render_report(graph, analysis);
    std::fs::write(path, content)?;
    Ok(())
}
