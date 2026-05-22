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

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use chrono::Local;
use graphify_build::Graph;
use regex::Regex;
use serde_json::Value;
use thiserror::Error;

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
fn safe_community_name(label: &str) -> String {
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
// Node classification helpers (mirrors Python `analyze._is_file_node` /
// `_is_concept_node`).  These live here because `graphify-analyze` is a stub
// and `report.py` calls them directly on `G`.
// ---------------------------------------------------------------------------

fn is_file_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return false;
    };
    let label = attrs
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if label.is_empty() {
        return false;
    }
    // File-level hub: label matches the source filename.
    let source_file = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !source_file.is_empty() {
        let filename = source_file.rsplit('/').next().unwrap_or(source_file);
        if label == filename {
            return true;
        }
    }
    // Method stub: AST extractor labels methods as `.method_name()`.
    if label.starts_with('.') && label.ends_with("()") {
        return true;
    }
    // Module-level function stub: `name()` with degree <= 1.
    if label.ends_with("()") && node_degree(graph, node_id) <= 1 {
        return true;
    }
    false
}

fn is_concept_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return true;
    };
    let source = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if source.is_empty() {
        return true;
    }
    // No file extension in the last path component → concept label, not a real file.
    let last = source.rsplit('/').next().unwrap_or(source);
    !last.contains('.')
}

/// Count how many edges involve `node_id` (undirected degree).
fn node_degree(graph: &Graph, node_id: &str) -> usize {
    graph
        .edges()
        .filter(|e| e.source == node_id || e.target == node_id)
        .count()
}

// ---------------------------------------------------------------------------
// Comma formatting — Python uses `f"{n:,}"` which inserts thousands commas.
// ---------------------------------------------------------------------------

fn fmt_comma(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
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
// Section renderers
// ---------------------------------------------------------------------------

fn render_corpus_check(
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

struct ConfidenceStats {
    ext_pct: u64,
    inf_pct: u64,
    amb_pct: u64,
    inf_edges_len: usize,
    inf_avg: Option<f64>,
}

fn render_summary(
    lines: &mut Vec<String>,
    graph: &Graph,
    communities: &Communities<'_>,
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
        communities.len()
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

fn render_freshness(lines: &mut Vec<String>, commit: &str) {
    let short = if commit.len() >= 8 {
        &commit[..8]
    } else {
        commit
    };
    lines.push(String::new());
    lines.push("## Graph Freshness".to_string());
    lines.push(format!("- Built from commit: `{short}`"));
    lines
        .push("- Run `git rev-parse HEAD` and compare to check if the graph is stale.".to_string());
    lines.push("- Run `graphify update .` after code changes (no API cost).".to_string());
}

fn render_nav_hubs(
    lines: &mut Vec<String>,
    non_empty: &[(i64, &Vec<&str>)],
    community_labels: &HashMap<i64, &str>,
) {
    lines.push(String::new());
    lines.push("## Community Hubs (Navigation)".to_string());
    for (cid, _) in non_empty {
        let label = community_labels
            .get(cid)
            .copied()
            .map_or_else(|| format!("Community {cid}"), ToString::to_string);
        let safe = safe_community_name(&label);
        lines.push(format!("- [[_COMMUNITY_{safe}|{label}]]"));
    }
}

fn render_god_nodes(lines: &mut Vec<String>, god_node_list: &[Value]) {
    lines.push(String::new());
    lines.push("## God Nodes (most connected - your core abstractions)".to_string());
    for (i, node) in god_node_list.iter().enumerate() {
        let label = node
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let degree = node.get("degree").and_then(Value::as_u64).unwrap_or(0);
        lines.push(format!("{}. `{label}` - {degree} edges", i + 1));
    }
}

fn render_surprising(lines: &mut Vec<String>, surprise_list: &[Value]) {
    lines.push(String::new());
    lines.push("## Surprising Connections (you probably didn't know these)".to_string());
    if surprise_list.is_empty() {
        lines.push(
            "- None detected - all connections are within the same source files.".to_string(),
        );
        return;
    }
    for s in surprise_list {
        let source = s.get("source").and_then(Value::as_str).unwrap_or_default();
        let target = s.get("target").and_then(Value::as_str).unwrap_or_default();
        let relation = s
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("related_to");
        let note = s.get("note").and_then(Value::as_str).unwrap_or_default();
        let src_files = s.get("source_files").and_then(Value::as_array);
        let src0 = src_files
            .and_then(|f| f.first())
            .and_then(Value::as_str)
            .unwrap_or_default();
        let src1 = src_files
            .and_then(|f| f.get(1))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let conf = s
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("EXTRACTED");
        let cscore = s.get("confidence_score").and_then(Value::as_f64);
        let conf_tag = if conf == "INFERRED" {
            if let Some(cs) = cscore {
                format!("INFERRED {cs:.2}")
            } else {
                conf.to_string()
            }
        } else {
            conf.to_string()
        };
        let sem_tag = if relation == "semantically_similar_to" {
            " [semantically similar]"
        } else {
            ""
        };
        lines.push(format!(
            "- `{source}` --{relation}--> `{target}`  [{conf_tag}]{sem_tag}"
        ));
        let note_part = if note.is_empty() {
            String::new()
        } else {
            format!("  _{note}_")
        };
        lines.push(format!("  {src0} → {src1}{note_part}"));
    }
}

fn render_hyperedges(lines: &mut Vec<String>, hyperedges: &[Value]) {
    lines.push(String::new());
    lines.push("## Hyperedges (group relationships)".to_string());
    for h in hyperedges {
        let node_labels = h
            .get("nodes")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let conf = h
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("INFERRED");
        let cscore = h.get("confidence_score").and_then(Value::as_f64);
        let conf_tag = if let Some(cs) = cscore {
            format!("{conf} {cs:.2}")
        } else {
            conf.to_string()
        };
        let label = h
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| h.get("id").and_then(Value::as_str))
            .unwrap_or_default();
        lines.push(format!("- **{label}** — {node_labels} [{conf_tag}]"));
    }
}

fn render_communities(
    lines: &mut Vec<String>,
    graph: &Graph,
    communities: &Communities<'_>,
    cohesion_scores: &HashMap<i64, f64>,
    community_labels: &HashMap<i64, &str>,
    thin_count_summary: usize,
    min_community_size: usize,
) {
    lines.push(String::new());
    lines.push(format!(
        "## Communities ({} total, {thin_count_summary} thin omitted)",
        communities.len()
    ));
    for (cid, nodes) in communities {
        let label = community_labels
            .get(cid)
            .copied()
            .map_or_else(|| format!("Community {cid}"), ToString::to_string);
        let score = cohesion_scores.get(cid).copied().unwrap_or(0.0);
        let real_nodes: Vec<&&str> = nodes.iter().filter(|n| !is_file_node(graph, n)).collect();
        if real_nodes.is_empty() || real_nodes.len() < min_community_size {
            continue;
        }
        let display: Vec<String> = real_nodes
            .iter()
            .take(8)
            .map(|n| {
                graph
                    .node_data(n)
                    .and_then(|a| a.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(n)
                    .to_string()
            })
            .collect();
        let suffix = if real_nodes.len() > 8 {
            format!(" (+{} more)", real_nodes.len() - 8)
        } else {
            String::new()
        };
        lines.push(String::new());
        lines.push(format!("### Community {cid} - \"{label}\""));
        lines.push(format!("Cohesion: {score:.2}"));
        lines.push(format!(
            "Nodes ({}): {}{}",
            real_nodes.len(),
            display.join(", "),
            suffix
        ));
    }
}

fn render_ambiguous(lines: &mut Vec<String>, graph: &Graph) {
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

fn render_gaps(
    lines: &mut Vec<String>,
    graph: &Graph,
    communities: &Communities<'_>,
    min_community_size: usize,
    amb_pct: u64,
) {
    let isolated: Vec<&str> = graph
        .nodes()
        .filter(|(id, attrs)| {
            node_degree(graph, id) <= 1
                && !is_file_node(graph, id)
                && !is_concept_node(graph, id)
                && attrs.get("file_type").and_then(Value::as_str) != Some("rationale")
        })
        .map(|(id, _)| id.as_str())
        .collect();

    let thin_community_count = communities
        .iter()
        .filter(|(_, nodes)| {
            let real = nodes.iter().filter(|n| !is_file_node(graph, n)).count();
            real > 0 && real < 3
        })
        .count();

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

fn render_questions(lines: &mut Vec<String>, suggested_questions: &[Value]) {
    lines.push(String::new());
    lines.push("## Suggested Questions".to_string());
    let no_signal = suggested_questions.len() == 1
        && suggested_questions
            .first()
            .and_then(|q| q.get("type"))
            .and_then(Value::as_str)
            == Some("no_signal");
    if no_signal {
        let why = suggested_questions
            .first()
            .and_then(|q| q.get("why"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        lines.push(format!("_{why}_"));
    } else {
        lines.push("_Questions this graph is uniquely positioned to answer:_".to_string());
        lines.push(String::new());
        for q in suggested_questions {
            if let Some(question) = q.get("question").and_then(Value::as_str)
                && !question.is_empty()
            {
                lines.push(format!("- **{question}**"));
                let why = q.get("why").and_then(Value::as_str).unwrap_or_default();
                lines.push(format!("  _{why}_"));
            }
        }
    }
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
        .filter(|(_, nodes)| nodes.iter().any(|n| !is_file_node(graph, n)))
        .map(|(cid, nodes)| (*cid, nodes))
        .collect();

    let thin_count_summary = communities
        .iter()
        .filter(|(_, nodes)| {
            let real = nodes.iter().filter(|n| !is_file_node(graph, n)).count();
            real > 0 && real < min_community_size
        })
        .count();
    let shown_count = communities.len() - thin_count_summary;

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
        &communities,
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
    render_gaps(&mut lines, graph, &communities, min_community_size, amb_pct);

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
