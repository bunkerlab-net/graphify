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

/// Input bundle for [`surprise_score`].
///
/// Groups the per-edge data needed to compute a surprise score so that the
/// function signature stays manageable.  Mirrors Python `_surprise_score`'s
/// parameter set.
pub struct SurpriseScoreInput<'a> {
    /// The graph the edge belongs to.
    pub graph: &'a Graph,
    /// Source node ID of the edge.
    pub u: &'a str,
    /// Target node ID of the edge.
    pub v: &'a str,
    /// Edge attribute map (confidence, relation, `_src`/`_tgt` overrides, …).
    pub data: &'a IndexMap<String, Value>,
    /// Pre-built `node_id → community_id` inversion (see `cross_lang::node_community_map`).
    pub node_community: &'a IndexMap<String, i64>,
    /// `source_file` attribute of node `u`.
    pub u_source: &'a str,
    /// `source_file` attribute of node `v`.
    pub v_source: &'a str,
    /// Pre-computed degree map, or `None` to compute on demand.
    ///
    /// Pass `Some(&degrees)` when scoring many edges to avoid recomputing
    /// degree counts for every call.
    pub degrees: Option<&'a IndexMap<String, usize>>,
}

/// Score how surprising a cross-file edge is.
///
/// Returns `(score, reasons)` where `score` is a non-negative integer (higher
/// = more surprising) and `reasons` is a human-readable list of contributing
/// factors.
///
/// Scoring factors include confidence level (AMBIGUOUS/INFERRED), cross-file-type
/// bonus, cross-repo bonus, cross-community bonus, semantic-similarity multiplier,
/// and a peripheral-to-hub bonus.
///
/// Mirrors Python `_surprise_score`.
#[must_use]
pub fn surprise_score(input: &SurpriseScoreInput<'_>) -> (i32, Vec<String>) {
    let SurpriseScoreInput {
        graph,
        u,
        v,
        data,
        node_community,
        u_source,
        v_source,
        degrees,
    } = *input;
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

/// Pre-computed context threaded through per-edge cross-file scoring.
struct CrossFileCtx<'a> {
    graph: &'a Graph,
    node_community: IndexMap<String, i64>,
    degrees: IndexMap<String, usize>,
    structural_relations: IndexSet<&'static str>,
}

/// Score a single cross-file edge or return `None` if it should be filtered out.
fn score_cross_file_edge(
    edge: &graphify_build::Edge,
    ctx: &CrossFileCtx<'_>,
) -> Option<(i32, Value)> {
    let u = edge.source.as_str();
    let v = edge.target.as_str();
    let data = &edge.attrs;

    let relation = data.get("relation").and_then(Value::as_str).unwrap_or("");
    if ctx.structural_relations.contains(relation) {
        return None;
    }
    if is_concept_node(ctx.graph, u) || is_concept_node(ctx.graph, v) {
        return None;
    }
    if is_file_node(ctx.graph, u, &ctx.degrees) || is_file_node(ctx.graph, v, &ctx.degrees) {
        return None;
    }

    let u_source = node_source_file(ctx.graph, u);
    let v_source = node_source_file(ctx.graph, v);
    if u_source.is_empty() || v_source.is_empty() || u_source == v_source {
        return None;
    }

    let (score, reasons) = surprise_score(&SurpriseScoreInput {
        graph: ctx.graph,
        u,
        v,
        data,
        node_community: &ctx.node_community,
        u_source,
        v_source,
        degrees: Some(&ctx.degrees),
    });

    let (src_id, tgt_id) = resolved_endpoints(ctx.graph, data, u, v);
    let entry = json!({
        "source": node_label(ctx.graph, src_id),
        "target": node_label(ctx.graph, tgt_id),
        "source_files": [node_source_file(ctx.graph, src_id), node_source_file(ctx.graph, tgt_id)],
        "confidence": data.get("confidence").and_then(Value::as_str).unwrap_or("EXTRACTED"),
        "relation": relation,
        "why": if reasons.is_empty() { "cross-file semantic connection".to_string() } else { reasons.join("; ") },
    });
    Some((score, entry))
}

/// Resolve `_src`/`_tgt` overrides (used for split-node aliases), falling back to `u`/`v`.
fn resolved_endpoints<'a>(
    graph: &Graph,
    data: &'a indexmap::IndexMap<String, Value>,
    u: &'a str,
    v: &'a str,
) -> (&'a str, &'a str) {
    let src = data
        .get("_src")
        .and_then(Value::as_str)
        .filter(|id| graph.contains_node(id))
        .unwrap_or(u);
    let tgt = data
        .get("_tgt")
        .and_then(Value::as_str)
        .filter(|id| graph.contains_node(id))
        .unwrap_or(v);
    (src, tgt)
}

fn node_label<'a>(graph: &'a Graph, id: &'a str) -> &'a str {
    graph
        .node_data(id)
        .and_then(|a| a.get("label"))
        .and_then(Value::as_str)
        .unwrap_or(id)
}

fn node_source_file<'a>(graph: &'a Graph, id: &str) -> &'a str {
    graph
        .node_data(id)
        .and_then(|a| a.get("source_file"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Find surprising connections for multi-file corpora.
///
/// Iterates all edges between nodes in different source files, scores each
/// via [`surprise_score`], and returns the top `top_n` by score.  Falls back
/// to [`cross_community_surprises`] when no cross-file edges are found.
///
/// Mirrors the multi-source branch of Python `surprising_connections`.
fn cross_file_surprises(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    top_n: usize,
) -> Vec<Value> {
    let ctx = CrossFileCtx {
        graph,
        node_community: node_community_map(communities),
        degrees: all_degrees(graph),
        structural_relations: ["imports", "imports_from", "contains", "method"]
            .into_iter()
            .collect(),
    };

    let mut candidates: Vec<(i32, Value)> = if graph.edge_list.len() >= PARALLEL_SURPRISE_THRESHOLD
    {
        graph
            .edge_list
            .par_iter()
            .filter_map(|e| score_cross_file_edge(e, &ctx))
            .collect()
    } else {
        graph
            .edges()
            .filter_map(|e| score_cross_file_edge(e, &ctx))
            .collect()
    };

    candidates.sort_by_key(|item| Reverse(item.0));
    let result: Vec<Value> = candidates.into_iter().map(|(_, v)| v).collect();

    if result.is_empty() {
        return cross_community_surprises(graph, communities, top_n);
    }
    result.into_iter().take(top_n).collect()
}

/// Edge betweenness fallback when no community data exists.
fn betweenness_fallback(graph: &Graph, top_n: usize) -> Vec<Value> {
    if graph.edge_count() == 0 || graph.node_count() > 5000 {
        return Vec::new();
    }
    let mut top_edges = edge_betweenness_centrality(graph);
    top_edges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    top_edges
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
        .collect()
}

/// Map a confidence string to a sort key so AMBIGUOUS edges sort first.
///
/// Lower values appear earlier when sorted ascending: AMBIGUOUS (0) →
/// INFERRED (1) → EXTRACTED (2) → unknown (3).
fn conf_order(c: &str) -> i32 {
    match c {
        "AMBIGUOUS" => 0,
        "INFERRED" => 1,
        "EXTRACTED" => 2,
        _ => 3,
    }
}

/// Pre-computed context threaded through per-edge cross-community scoring.
struct CrossCommunityCtx<'a> {
    graph: &'a Graph,
    node_community: IndexMap<String, i64>,
    degrees: IndexMap<String, usize>,
    structural_relations: IndexSet<&'static str>,
}

/// Score a single cross-community edge or return `None` to filter it out.
fn score_cross_community_edge(
    edge: &graphify_build::Edge,
    ctx: &CrossCommunityCtx<'_>,
) -> Option<(i32, (i64, i64), Value)> {
    let u = edge.source.as_str();
    let v = edge.target.as_str();
    let data = &edge.attrs;

    let cu = ctx.node_community.get(u).copied()?;
    let cv = ctx.node_community.get(v).copied()?;
    if cu == cv {
        return None;
    }
    if is_file_node(ctx.graph, u, &ctx.degrees) || is_file_node(ctx.graph, v, &ctx.degrees) {
        return None;
    }
    let relation = data.get("relation").and_then(Value::as_str).unwrap_or("");
    if ctx.structural_relations.contains(relation) {
        return None;
    }
    let confidence = data
        .get("confidence")
        .and_then(Value::as_str)
        .unwrap_or("EXTRACTED");

    let (src_id, tgt_id) = resolved_endpoints(ctx.graph, data, u, v);
    let pair = if cu <= cv { (cu, cv) } else { (cv, cu) };
    let entry = json!({
        "source": node_label(ctx.graph, src_id),
        "target": node_label(ctx.graph, tgt_id),
        "source_files": [node_source_file(ctx.graph, src_id), node_source_file(ctx.graph, tgt_id)],
        "confidence": confidence,
        "relation": relation,
        "note": format!("Bridges community {cu} \u{2192} community {cv}"),
    });
    Some((conf_order(confidence), pair, entry))
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
fn cross_community_surprises(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    top_n: usize,
) -> Vec<Value> {
    if communities.is_empty() {
        return betweenness_fallback(graph, top_n);
    }

    let ctx = CrossCommunityCtx {
        graph,
        node_community: node_community_map(communities),
        degrees: all_degrees(graph),
        structural_relations: ["imports", "imports_from", "contains", "method"]
            .into_iter()
            .collect(),
    };

    let mut surprises: Vec<(i32, (i64, i64), Value)> =
        if graph.edge_list.len() >= PARALLEL_SURPRISE_THRESHOLD {
            graph
                .edge_list
                .par_iter()
                .filter_map(|e| score_cross_community_edge(e, &ctx))
                .collect()
        } else {
            graph
                .edges()
                .filter_map(|e| score_cross_community_edge(e, &ctx))
                .collect()
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
