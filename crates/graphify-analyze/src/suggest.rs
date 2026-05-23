//! Question suggestion from graph structure.
//!
//! Extracted from `lib.rs` to isolate `suggest_questions`, which generates
//! LLM prompts derived from AMBIGUOUS edges, bridge nodes, inferred
//! relationships, isolated nodes, and low-cohesion communities.

use graphify_build::Graph;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};
use std::cmp::Reverse;

use crate::centrality::{all_degrees, betweenness_centrality, neighbors};
use crate::classify::{is_concept_node, is_file_node};
use crate::cross_lang::node_community_map;

/// Single-pass cohesion scores for every community.
///
/// Builds a `node → community_id` map, walks the edge list once counting
/// intra-community edges per community, then divides by the maximum
/// possible edges. Runs in `O(N + E + C)` instead of the per-community
/// `cohesion_score` call's `O(C × E)`.
fn precompute_cohesion(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
) -> IndexMap<i64, f64> {
    if communities.is_empty() {
        return IndexMap::new();
    }
    let mut node_to_cid: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for (&cid, nodes) in communities {
        for n in nodes {
            node_to_cid.insert(n.as_str(), cid);
        }
    }
    let directed = graph.kind.is_directed();
    let mut actual: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let mut seen_directed: std::collections::HashSet<(i64, &str, &str)> =
        std::collections::HashSet::new();
    for edge in graph.edges() {
        let src = edge.source.as_str();
        let tgt = edge.target.as_str();
        let (Some(&cu), Some(&cv)) = (node_to_cid.get(src), node_to_cid.get(tgt)) else {
            continue;
        };
        if cu != cv {
            continue;
        }
        if directed {
            let (a, b) = if src <= tgt { (src, tgt) } else { (tgt, src) };
            if !seen_directed.insert((cu, a, b)) {
                continue;
            }
        }
        *actual.entry(cu).or_insert(0) += 1;
    }
    communities
        .iter()
        .map(|(&cid, nodes)| {
            let n = nodes.len();
            if n <= 1 {
                return (cid, 1.0);
            }
            #[allow(clippy::cast_precision_loss)]
            let possible = (n * (n - 1)) as f64 / 2.0;
            #[allow(clippy::cast_precision_loss)]
            let actual_f = actual.get(&cid).copied().unwrap_or(0) as f64;
            let score = if possible > 0.0 {
                actual_f / possible
            } else {
                0.0
            };
            (cid, score)
        })
        .collect()
}

/// Generate questions the graph is uniquely positioned to answer.
///
/// Based on: AMBIGUOUS edges, bridge nodes, underexplored god nodes, isolated
/// nodes, and low-cohesion communities.
///
/// Mirrors Python `suggest_questions`.
#[must_use]
#[allow(clippy::too_many_lines)] // five distinct scoring categories; splitting would obscure flow
pub fn suggest_questions(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    community_labels: &IndexMap<i64, String>,
    top_n: usize,
) -> Vec<Value> {
    let node_community = node_community_map(communities);
    let mut questions: Vec<Value> = Vec::new();
    let perf = std::env::var("GRAPHIFY_PERF_LOG").is_ok();

    // Precompute degrees once — `is_file_node` requires a `degrees` map, and
    // we re-use it across sections 2/3/4 to avoid recomputing.
    let degrees = all_degrees(graph);

    let t = std::time::Instant::now();
    // 1. AMBIGUOUS edges → unresolved relationship questions
    for edge in graph.edges() {
        let data = &edge.attrs;
        if data.get("confidence").and_then(Value::as_str) == Some("AMBIGUOUS") {
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
            let relation = data
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("related to");
            questions.push(json!({
                "type": "ambiguous_edge",
                "question": format!("What is the exact relationship between `{ul}` and `{vl}`?"),
                "why": format!("Edge tagged AMBIGUOUS (relation: {relation}) - confidence is low."),
            }));
        }
    }

    if perf {
        eprintln!(
            "[perf]     suggest_questions/s1_ambiguous: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    // 2. Bridge nodes (high betweenness) → cross-cutting concern questions
    if graph.edge_count() > 0 {
        let k = if graph.node_count() > 1000 {
            Some(100_usize.min(graph.node_count()))
        } else {
            None
        };
        let betweenness = betweenness_centrality(graph, k);
        let mut bridges: Vec<(&str, f64)> = betweenness
            .iter()
            .filter_map(|(node_id, &sc)| {
                if !is_file_node(graph, node_id, &degrees)
                    && !is_concept_node(graph, node_id)
                    && sc > 0.0
                {
                    Some((node_id.as_str(), sc))
                } else {
                    None
                }
            })
            .collect();
        bridges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        bridges.truncate(3);

        for (node_id, sc) in bridges {
            let label = graph
                .node_data(node_id)
                .and_then(|a| a.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(node_id);
            let cid = node_community.get(node_id).copied();
            let comm_label = cid
                .and_then(|c| community_labels.get(&c))
                .cloned()
                .unwrap_or_else(|| {
                    cid.map_or_else(|| "unknown".to_string(), |c| format!("Community {c}"))
                });
            let nbrs = neighbors(graph, node_id);
            let neighbor_comms: IndexSet<i64> = nbrs
                .iter()
                .filter_map(|&n| node_community.get(n).copied())
                .filter(|&c| Some(c) != cid)
                .collect();
            if !neighbor_comms.is_empty() {
                let other_labels: Vec<String> = neighbor_comms
                    .iter()
                    .map(|c| {
                        community_labels
                            .get(c)
                            .cloned()
                            .unwrap_or_else(|| format!("Community {c}"))
                    })
                    .collect();
                let other_str = other_labels
                    .iter()
                    .map(|l| format!("`{l}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                questions.push(json!({
                    "type": "bridge_node",
                    "question": format!("Why does `{label}` connect `{comm_label}` to {other_str}?"),
                    "why": format!("High betweenness centrality ({sc:.3}) - this node is a cross-community bridge."),
                }));
            }
        }
    }

    if perf {
        eprintln!(
            "[perf]     suggest_questions/s2_bridges: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    // 3. God nodes with many INFERRED edges → verification questions
    let mut top_nodes: Vec<(&str, usize)> = degrees
        .iter()
        .filter_map(|(id, &d)| {
            if is_file_node(graph, id, &degrees) {
                None
            } else {
                Some((id.as_str(), d))
            }
        })
        .collect();
    top_nodes.sort_by_key(|item| Reverse(item.1));
    top_nodes.truncate(5);

    // Pre-bucket edges by their endpoints once so the per-top-node lookup
    // below is O(1) instead of an O(E) scan per node (5 nodes × 36k edges
    // was the previous shape).
    let mut edges_by_node: std::collections::HashMap<&str, Vec<&graphify_build::Edge>> =
        std::collections::HashMap::new();
    for edge in graph.edges() {
        edges_by_node
            .entry(edge.source.as_str())
            .or_default()
            .push(edge);
        if edge.target != edge.source {
            edges_by_node
                .entry(edge.target.as_str())
                .or_default()
                .push(edge);
        }
    }

    for (node_id, _) in top_nodes {
        let inferred: Vec<(&str, &str, &IndexMap<String, Value>)> = edges_by_node
            .get(node_id)
            .into_iter()
            .flatten()
            .filter(|e| e.attrs.get("confidence").and_then(Value::as_str) == Some("INFERRED"))
            .map(|e| (e.source.as_str(), e.target.as_str(), &e.attrs))
            .collect();

        if inferred.len() >= 2 {
            let label = graph
                .node_data(node_id)
                .and_then(|a| a.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(node_id);
            let mut others: Vec<String> = Vec::new();
            for &(u, v, d) in &inferred[..2] {
                let src_id = d
                    .get("_src")
                    .and_then(Value::as_str)
                    .filter(|id| graph.contains_node(id))
                    .unwrap_or(u);
                let tgt_id = d
                    .get("_tgt")
                    .and_then(Value::as_str)
                    .filter(|id| graph.contains_node(id))
                    .unwrap_or(v);
                let other_id = if src_id == node_id { tgt_id } else { src_id };
                let other_label = graph
                    .node_data(other_id)
                    .and_then(|a| a.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(other_id);
                others.push(other_label.to_string());
            }
            let count = inferred.len();
            questions.push(json!({
                "type": "verify_inferred",
                "question": format!("Are the {count} inferred relationships involving `{label}` (e.g. with `{}` and `{}`) actually correct?", others[0], others[1]),
                "why": format!("`{label}` has {count} INFERRED edges - model-reasoned connections that need verification."),
            }));
        }
    }

    if perf {
        eprintln!(
            "[perf]     suggest_questions/s3_gods: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    // 4. Isolated or weakly-connected nodes → exploration questions.
    // Reuse the `degrees` map computed at function entry rather than
    // recomputing.
    let isolated: Vec<&str> = graph
        .nodes()
        .filter_map(|(id, _)| {
            if degrees.get(id).copied().unwrap_or(0) <= 1
                && !is_file_node(graph, id, &degrees)
                && !is_concept_node(graph, id)
            {
                Some(id.as_str())
            } else {
                None
            }
        })
        .collect();

    if !isolated.is_empty() {
        let labels: Vec<String> = isolated[..3.min(isolated.len())]
            .iter()
            .map(|&id| {
                graph
                    .node_data(id)
                    .and_then(|a| a.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_string()
            })
            .collect();
        let label_str = labels
            .iter()
            .map(|l| format!("`{l}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let count = isolated.len();
        questions.push(json!({
            "type": "isolated_nodes",
            "question": format!("What connects {label_str} to the rest of the system?"),
            "why": format!("{count} weakly-connected nodes found - possible documentation gaps or missing edges."),
        }));
    }

    if perf {
        eprintln!(
            "[perf]     suggest_questions/s4_isolated: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    let t = std::time::Instant::now();
    // 5. Low-cohesion communities → structural questions.
    //
    // The naive shape called `cohesion_score(graph, nodes)` per community,
    // each rescanning the full edge list — `O(C × E)` total. For a graph
    // with 785 communities and 36k edges that's 28M edge iterations.
    // Precompute per-community intra-edge counts in one pass and consult
    // that map instead.
    let cohesion_scores = precompute_cohesion(graph, communities);
    for (cid, nodes) in communities {
        let score = cohesion_scores.get(cid).copied().unwrap_or(0.0);
        if score < 0.15 && nodes.len() >= 5 {
            let label = community_labels
                .get(cid)
                .cloned()
                .unwrap_or_else(|| format!("Community {cid}"));
            questions.push(json!({
                "type": "low_cohesion",
                "question": format!("Should `{label}` be split into smaller, more focused modules?"),
                "why": format!("Cohesion score {score} - nodes in this community are weakly interconnected."),
            }));
        }
    }

    if perf {
        eprintln!(
            "[perf]     suggest_questions/s5_cohesion: {:.2}s",
            t.elapsed().as_secs_f64()
        );
    }
    if questions.is_empty() {
        return vec![json!({
            "type": "no_signal",
            "question": null,
            "why": "Not enough signal to generate questions. This usually means the corpus has no AMBIGUOUS edges, no bridge nodes, no INFERRED relationships, and all communities are tightly cohesive. Add more files or run with --mode deep to extract richer edges.",
        })];
    }

    questions.into_iter().take(top_n).collect()
}
