//! The top-level [`render_report`] and [`write_report`] drivers.

use std::path::Path;

use chrono::Local;
use serde_json::Value;

use graphify_build::Graph;

use crate::analysis::{Communities, extract_cohesion, extract_communities, extract_labels};
use crate::error::ReportError;
use crate::sections;
use crate::sections::{
    communities::{render_communities, render_nav_hubs},
    detection::{
        ConfidenceStats, render_ambiguous, render_corpus_check, render_gaps, render_summary,
    },
    god_nodes::render_god_nodes,
    header::render_freshness,
    suggestions::render_questions,
    surprises::{render_hyperedges, render_surprising},
};

/// Render a `GRAPH_REPORT.md` string from a graph and analysis result.
///
/// The output is byte-identical to the Python `generate()` function for
/// the same inputs.
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

    // Precompute degrees once — `is_file_node` and isolated-node detection
    // both need them. Without this, every per-node call iterates the full
    // edge list (`O(N × E)` total) and dominates report time on large graphs.
    let degrees = sections::compute_degrees(graph);

    let non_empty: Vec<(i64, &Vec<&str>)> = communities
        .iter()
        .filter(|(_, nodes)| {
            nodes
                .iter()
                .any(|n| !sections::is_file_node(graph, n, &degrees))
        })
        .map(|(cid, nodes)| (*cid, nodes))
        .collect();

    let thin_count_summary = communities
        .iter()
        .filter(|(_, nodes)| {
            let real = nodes
                .iter()
                .filter(|n| !sections::is_file_node(graph, n, &degrees))
                .count();
            real > 0 && real < min_community_size
        })
        .count();
    let shown_count = communities.len() - thin_count_summary;

    let isolated: Vec<&str> = graph
        .nodes()
        .filter(|(id, attrs)| {
            degrees.get(id.as_str()).copied().unwrap_or(0) <= 1
                && !sections::is_file_node(graph, id, &degrees)
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
                .filter(|n| !sections::is_file_node(graph, n, &degrees))
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
        &degrees,
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
