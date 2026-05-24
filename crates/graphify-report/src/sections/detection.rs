//! Detection-result stats block and summary section.
//!
//! Extracted from `lib.rs` to group all renderers that operate on
//! `detection_result`, edge-confidence statistics, and the summary line.

use graphify_build::Graph;
use serde_json::Value;

use super::tokens::fmt_comma;

/// Confidence statistics derived from graph edge attributes.
pub(crate) struct ConfidenceStats {
    /// Percentage of edges with `confidence == "EXTRACTED"`.
    pub ext_pct: u64,
    /// Percentage of edges with `confidence == "INFERRED"`.
    pub inf_pct: u64,
    /// Percentage of edges with `confidence == "AMBIGUOUS"`.
    pub amb_pct: u64,
    /// Raw count of `"INFERRED"` edges (used for the avg-confidence annotation).
    pub inf_edges_len: usize,
    /// Mean confidence score across all `"INFERRED"` edges, or `None` if there are none.
    pub inf_avg: Option<f64>,
}

/// Render the "Corpus Check" section from detection-result data.
pub(crate) fn render_corpus_check(
    lines: &mut Vec<String>,
    detection: Option<&serde_json::Map<String, Value>>,
) {
    lines.push("## Corpus Check".to_string());
    if let Some(det) = detection {
        if let Some(warning) = det.get("warning").and_then(Value::as_str) {
            lines.push(format!("- {warning}"));
        } else {
            let total_files = det.get("total_files").and_then(Value::as_u64).unwrap_or(0);
            let total_words = det.get("total_words").and_then(Value::as_u64).unwrap_or(0);
            lines.push(format!(
                "- {} files · ~{} words",
                total_files,
                fmt_comma(total_words)
            ));
            lines.push(
                "- Verdict: corpus is large enough that graph structure adds value.".to_string(),
            );
        }
    }
}

/// Render the "Summary" section.
pub(crate) fn render_summary(
    lines: &mut Vec<String>,
    graph: &Graph,
    community_count: usize,
    thin_count_summary: usize,
    shown_count: usize,
    stats: &ConfidenceStats,
    token_cost: Option<&serde_json::Map<String, Value>>,
) {
    let ConfidenceStats {
        ext_pct,
        inf_pct,
        amb_pct,
        inf_edges_len,
        inf_avg,
    } = stats;
    lines.push(String::new());
    lines.push("## Summary".to_string());

    let base = format!(
        "- {} nodes · {} edges · {} communities",
        graph.node_count(),
        graph.edge_count(),
        community_count
    );
    let thin_suffix = if thin_count_summary > 0 {
        format!(" ({shown_count} shown, {thin_count_summary} thin omitted)")
    } else {
        String::new()
    };
    lines.push(format!("{base}{thin_suffix}"));

    let extraction = {
        let base = format!(
            "- Extraction: {ext_pct}% EXTRACTED · {inf_pct}% INFERRED · {amb_pct}% AMBIGUOUS"
        );
        if let Some(avg) = inf_avg {
            format!("{base} · INFERRED: {inf_edges_len} edges (avg confidence: {avg})")
        } else {
            base
        }
    };
    lines.push(extraction);

    let (inp, out) = token_cost.map_or((0, 0), |tc| {
        (
            tc.get("input").and_then(Value::as_u64).unwrap_or(0),
            tc.get("output").and_then(Value::as_u64).unwrap_or(0),
        )
    });
    lines.push(format!(
        "- Token cost: {} input · {} output",
        fmt_comma(inp),
        fmt_comma(out)
    ));
}

/// Render the "Ambiguous Edges" section when AMBIGUOUS edges exist.
pub(crate) fn render_ambiguous(lines: &mut Vec<String>, graph: &Graph) {
    let ambiguous: Vec<&graphify_build::Edge> = graph
        .edges()
        .filter(|e| e.attrs.get("confidence").and_then(Value::as_str) == Some("AMBIGUOUS"))
        .collect();
    if ambiguous.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push("## Ambiguous Edges - Review These".to_string());
    for edge in &ambiguous {
        let ul = graph
            .node_data(&edge.source)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(&edge.source);
        let vl = graph
            .node_data(&edge.target)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(&edge.target);
        lines.push(format!("- `{ul}` → `{vl}`  [AMBIGUOUS]"));
        let sf = edge
            .attrs
            .get("source_file")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let rel = edge
            .attrs
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        lines.push(format!("  {sf} · relation: {rel}"));
    }
}

/// Render the "Knowledge Gaps" section.
///
/// `isolated` is pre-filtered by the caller; `thin_community_count` is the
/// number of communities below `min_community_size`.
pub(crate) fn render_gaps(
    lines: &mut Vec<String>,
    graph: &Graph,
    thin_community_count: usize,
    isolated: &[&str],
    min_community_size: usize,
    amb_pct: u64,
) {
    let gap_count = isolated.len() + thin_community_count;
    if gap_count == 0 && amb_pct <= 20 {
        return;
    }

    lines.push(String::new());
    lines.push("## Knowledge Gaps".to_string());

    if !isolated.is_empty() {
        let isolated_labels: Vec<String> = isolated
            .iter()
            .take(5)
            .map(|n| {
                graph
                    .node_data(n)
                    .and_then(|a| a.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(n)
                    .to_string()
            })
            .collect();
        let suffix = if isolated.len() > 5 {
            format!(" (+{} more)", isolated.len() - 5)
        } else {
            String::new()
        };
        let joined = isolated_labels
            .iter()
            .map(|l| format!("`{l}`"))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "- **{} isolated node(s):** {joined}{suffix}",
            isolated.len()
        ));
        lines.push(
            "  These have ≤1 connection - possible missing edges or undocumented components."
                .to_string(),
        );
    }
    if thin_community_count > 0 {
        lines.push(format!(
            "- **{thin_community_count} thin communities (<{min_community_size} nodes) omitted from report** — run `graphify query` to explore isolated nodes."
        ));
    }
    if amb_pct > 20 {
        lines.push(format!(
            "- **High ambiguity: {amb_pct}% of edges are AMBIGUOUS.** Review the Ambiguous Edges section above."
        ));
    }
}
