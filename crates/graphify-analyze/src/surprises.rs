//! Surprising-connection detection (cross-file and cross-community edges).
//!
//! Extracted from `lib.rs` to isolate `surprising_connections`,
//! `surprise_score`, `cross_file_surprises`, and `cross_community_surprises`.

use graphify_build::Graph;
use indexmap::{IndexMap, IndexSet};
use rayon::prelude::*;
use serde_json::{Value, json};
use std::cmp::Reverse;

use crate::centrality::{all_degrees, edge_betweenness_centrality};
use crate::classify::{file_category, is_concept_node, is_file_node, top_level_dir};
use crate::cross_lang::{cross_language, node_community_map};

/// Edge-count threshold above which surprise scoring is dispatched to Rayon.
const PARALLEL_SURPRISE_THRESHOLD: usize = 256;

/// Score how surprising a cross-file edge is.
///
/// Returns `(score, reasons)`.
///
/// Mirrors Python `_surprise_score`.
///
/// # Errors
///
/// This function is infallible; it returns a plain `(i32, Vec<String>)`.
#[must_use]
#[allow(clippy::too_many_arguments)] // mirrors the Python _surprise_score signature 1:1
pub fn surprise_score(
    graph: &Graph,
    u: &str,
    v: &str,
    data: &IndexMap<String, Value>,
    node_community: &IndexMap<String, i64>,
    u_source: &str,
    v_source: &str,
    degrees: Option<&IndexMap<String, usize>>,
) -> (i32, Vec<String>) {
    let mut score: i32 = 0;
    let mut reasons: Vec<String> = Vec::new();

    let conf = data
        .get("confidence")
        .and_then(Value::as_str)
        .unwrap_or("EXTRACTED");
    let relation = data.get("relation").and_then(Value::as_str).unwrap_or("");

    let conf_bonus: i32 = match conf {
        "AMBIGUOUS" => 3,
        "INFERRED" => 2,
        _ => 1, // EXTRACTED and unknown types
    };

    let cat_u = file_category(u_source);
    let cat_v = file_category(v_source);

    // Suppress structural bonuses for INFERRED calls/uses that cross language
    // boundaries or connect code to a doc file.
    let suppress_structural = conf == "INFERRED"
        && (relation == "calls" || relation == "uses")
        && (cross_language(u_source, v_source)
            || ((cat_u == "code") != (cat_v == "code") && (cat_u == "doc" || cat_v == "doc")));

    let conf_bonus = if suppress_structural { 0 } else { conf_bonus };

    score += conf_bonus;
    if conf == "AMBIGUOUS" || conf == "INFERRED" {
        reasons.push(format!(
            "{} connection - not explicitly stated in source",
            conf.to_lowercase()
        ));
    }

    // Cross file-type bonus
    if cat_u != cat_v && !suppress_structural {
        score += 2;
        reasons.push(format!("crosses file types ({cat_u} \u{2194} {cat_v})"));
    }

    // Cross-repo bonus
    if top_level_dir(u_source) != top_level_dir(v_source) && !suppress_structural {
        score += 2;
        reasons.push("connects across different repos/directories".to_string());
    }

    // Cross-community bonus
    let cid_u = node_community.get(u).copied();
    let cid_v = node_community.get(v).copied();
    if let (Some(cu), Some(cv)) = (cid_u, cid_v)
        && cu != cv
        && !suppress_structural
    {
        score += 1;
        reasons.push("bridges separate communities".to_string());
    }

    // Semantic similarity bonus
    if relation == "semantically_similar_to" {
        #[allow(clippy::cast_possible_truncation)] // score fits in i32 after ×1.5
        let new_score = (f64::from(score) * 1.5) as i32;
        score = new_score;
        reasons.push("semantically similar concepts with no structural link".to_string());
    }

    // Peripheral→hub bonus
    let precomputed_deg_u: Option<usize>;
    let precomputed_deg_v: Option<usize>;
    let deg_u;
    let deg_v;
    if let Some(degs) = degrees {
        precomputed_deg_u = degs.get(u).copied();
        precomputed_deg_v = degs.get(v).copied();
        deg_u = precomputed_deg_u.unwrap_or(0);
        deg_v = precomputed_deg_v.unwrap_or(0);
    } else {
        let all = all_degrees(graph);
        deg_u = all.get(u).copied().unwrap_or(0);
        deg_v = all.get(v).copied().unwrap_or(0);
    }
    if deg_u.min(deg_v) <= 2 && deg_u.max(deg_v) >= 5 {
        score += 1;
        let (peripheral_id, hub_id) = if deg_u <= 2 { (u, v) } else { (v, u) };
        let peripheral = graph
            .node_data(peripheral_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(peripheral_id);
        let hub = graph
            .node_data(hub_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(hub_id);
        reasons.push(format!(
            "peripheral node `{peripheral}` unexpectedly reaches hub `{hub}`"
        ));
    }

    (score, reasons)
}

/// Find surprising connections for multi-file corpora.
///
/// Iterates all edges between nodes in different source files, scores each
/// via [`surprise_score`], and returns the top `top_n` by score.  Falls back
/// to [`cross_community_surprises`] when no cross-file edges are found.
///
/// Mirrors the multi-source branch of Python `surprising_connections`.
#[allow(clippy::too_many_lines)] // closure-heavy edge scorer; splitting would obscure flow.
fn cross_file_surprises(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    top_n: usize,
) -> Vec<Value> {
    let node_community = node_community_map(communities);
    let degrees = all_degrees(graph);

    let structural_relations: IndexSet<&str> = ["imports", "imports_from", "contains", "method"]
        .into_iter()
        .collect();

    // Per-edge scoring is read-only over `graph` and produces an owned tuple,
    // so it parallelises cleanly. Sort restores deterministic ordering.
    let score_edge = |edge: &graphify_build::Edge| -> Option<(i32, Value)> {
        let u = edge.source.as_str();
        let v = edge.target.as_str();
        let data = &edge.attrs;

        let relation = data.get("relation").and_then(Value::as_str).unwrap_or("");
        if structural_relations.contains(relation) {
            return None;
        }
        if is_concept_node(graph, u) || is_concept_node(graph, v) {
            return None;
        }
        if is_file_node(graph, u, &degrees) || is_file_node(graph, v, &degrees) {
            return None;
        }

        let u_source = graph
            .node_data(u)
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let v_source = graph
            .node_data(v)
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str)
            .unwrap_or("");

        if u_source.is_empty() || v_source.is_empty() || u_source == v_source {
            return None;
        }

        let (score, reasons) = surprise_score(
            graph,
            u,
            v,
            data,
            &node_community,
            u_source,
            v_source,
            Some(&degrees),
        );

        let src_id = data
            .get("_src")
            .and_then(Value::as_str)
            .filter(|id| graph.contains_node(id))
            .unwrap_or(u);
        let tgt_id = data
            .get("_tgt")
            .and_then(Value::as_str)
            .filter(|id| graph.contains_node(id))
            .unwrap_or(v);

        let src_label = graph
            .node_data(src_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(src_id);
        let tgt_label = graph
            .node_data(tgt_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(tgt_id);
        let src_file = graph
            .node_data(src_id)
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let tgt_file = graph
            .node_data(tgt_id)
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str)
            .unwrap_or("");

        let why = if reasons.is_empty() {
            "cross-file semantic connection".to_string()
        } else {
            reasons.join("; ")
        };

        Some((
            score,
            json!({
                "source": src_label,
                "target": tgt_label,
                "source_files": [src_file, tgt_file],
                "confidence": data.get("confidence").and_then(Value::as_str).unwrap_or("EXTRACTED"),
                "relation": relation,
                "why": why,
            }),
        ))
    };

    let mut candidates: Vec<(i32, Value)> = if graph.edge_list.len() >= PARALLEL_SURPRISE_THRESHOLD
    {
        graph.edge_list.par_iter().filter_map(score_edge).collect()
    } else {
        graph.edges().filter_map(score_edge).collect()
    };

    candidates.sort_by_key(|item| Reverse(item.0));
    let result: Vec<Value> = candidates.into_iter().map(|(_, v)| v).collect();

    if result.is_empty() {
        return cross_community_surprises(graph, communities, top_n);
    }
    result.into_iter().take(top_n).collect()
}

/// Find surprising connections for single-source corpora.
///
/// When the entire corpus comes from one file (no cross-file edges), community
/// boundaries serve as the next-best surprise signal.  Edges that cross two
/// different communities are deduplicated to one representative per community
/// pair, ordered by confidence (AMBIGUOUS first).  Falls back to edge
/// betweenness centrality when no community data is available.
///
/// Mirrors the single-source branch of Python `surprising_connections`.
#[allow(clippy::too_many_lines)] // algorithm has many branch cases; splitting would obscure flow
fn cross_community_surprises(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    top_n: usize,
) -> Vec<Value> {
    if communities.is_empty() {
        // Fall back to edge betweenness centrality
        if graph.edge_count() == 0 {
            return Vec::new();
        }
        if graph.node_count() > 5000 {
            return Vec::new();
        }
        let mut top_edges = edge_betweenness_centrality(graph);
        top_edges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let result = top_edges
            .into_iter()
            .take(top_n)
            .map(|((u, v), score_val)| {
                let u_attrs = graph.node_data(&u);
                let v_attrs = graph.node_data(&v);
                let data = graph.edge_data(&u, &v);
                json!({
                    "source": u_attrs.and_then(|a| a.get("label")).and_then(Value::as_str).unwrap_or(u.as_str()),
                    "target": v_attrs.and_then(|a| a.get("label")).and_then(Value::as_str).unwrap_or(v.as_str()),
                    "source_files": [
                        u_attrs.and_then(|a| a.get("source_file")).and_then(Value::as_str).unwrap_or(""),
                        v_attrs.and_then(|a| a.get("source_file")).and_then(Value::as_str).unwrap_or(""),
                    ],
                    "confidence": data.and_then(|d| d.get("confidence")).and_then(Value::as_str).unwrap_or("EXTRACTED"),
                    "relation": data.and_then(|d| d.get("relation")).and_then(Value::as_str).unwrap_or(""),
                    "note": format!("Bridges graph structure (betweenness={score_val:.3})"),
                })
            })
            .collect();
        return result;
    }

    let node_community = node_community_map(communities);
    let degrees = all_degrees(graph);
    let structural_relations: IndexSet<&str> = ["imports", "imports_from", "contains", "method"]
        .into_iter()
        .collect();

    // Confidence ordering: AMBIGUOUS < INFERRED < EXTRACTED
    let conf_order = |c: &str| -> i32 {
        match c {
            "AMBIGUOUS" => 0,
            "INFERRED" => 1,
            "EXTRACTED" => 2,
            _ => 3,
        }
    };

    // Per-edge scoring is read-only — fan out across Rayon. The downstream
    // `sort_by_key` makes ordering deterministic regardless of fan-in order.
    let score_edge = |edge: &graphify_build::Edge| -> Option<(i32, (i64, i64), Value)> {
        let u = edge.source.as_str();
        let v = edge.target.as_str();
        let data = &edge.attrs;

        let cid_u = node_community.get(u).copied();
        let cid_v = node_community.get(v).copied();
        let (Some(cu), Some(cv)) = (cid_u, cid_v) else {
            return None;
        };
        if cu == cv {
            return None;
        }
        if is_file_node(graph, u, &degrees) || is_file_node(graph, v, &degrees) {
            return None;
        }
        let relation = data.get("relation").and_then(Value::as_str).unwrap_or("");
        if structural_relations.contains(relation) {
            return None;
        }

        let confidence = data
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("EXTRACTED");

        let src_id = data
            .get("_src")
            .and_then(Value::as_str)
            .filter(|id| graph.contains_node(id))
            .unwrap_or(u);
        let tgt_id = data
            .get("_tgt")
            .and_then(Value::as_str)
            .filter(|id| graph.contains_node(id))
            .unwrap_or(v);

        let src_label = graph
            .node_data(src_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(src_id);
        let tgt_label = graph
            .node_data(tgt_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(tgt_id);
        let src_file = graph
            .node_data(src_id)
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let tgt_file = graph
            .node_data(tgt_id)
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str)
            .unwrap_or("");

        let pair = if cu <= cv { (cu, cv) } else { (cv, cu) };
        Some((
            conf_order(confidence),
            pair,
            json!({
                "source": src_label,
                "target": tgt_label,
                "source_files": [src_file, tgt_file],
                "confidence": confidence,
                "relation": relation,
                "note": format!("Bridges community {cu} \u{2192} community {cv}"),
            }),
        ))
    };

    let mut surprises: Vec<(i32, (i64, i64), Value)> =
        if graph.edge_list.len() >= PARALLEL_SURPRISE_THRESHOLD {
            graph.edge_list.par_iter().filter_map(score_edge).collect()
        } else {
            graph.edges().filter_map(score_edge).collect()
        };

    // Sort by confidence order (AMBIGUOUS first)
    surprises.sort_by_key(|(order, _, _)| *order);

    // Deduplicate by community pair — one edge per (A→B) boundary
    let mut seen_pairs: IndexSet<(i64, i64)> = IndexSet::new();
    let mut deduped: Vec<Value> = Vec::new();
    for (_, pair, val) in surprises {
        if seen_pairs.insert(pair) {
            deduped.push(val);
        }
    }
    deduped.into_iter().take(top_n).collect()
}

/// Find connections that are genuinely surprising.
///
/// For multi-file corpora: cross-file edges between real entities.
/// For single-file corpora: cross-community edges.
///
/// Mirrors Python `surprising_connections`.
#[must_use]
pub fn surprising_connections(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    top_n: usize,
) -> Vec<Value> {
    // Determine if this is a multi-source corpus
    let source_files: IndexSet<&str> = graph
        .nodes()
        .filter_map(|(_, attrs)| attrs.get("source_file").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .collect();
    let is_multi_source = source_files.len() > 1;

    if is_multi_source {
        cross_file_surprises(graph, communities, top_n)
    } else {
        cross_community_surprises(graph, communities, top_n)
    }
}
