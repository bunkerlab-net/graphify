//! Graph analysis.
//!
//! Ports `graphify-py/graphify/analyze.py`.
//!
//! Computes graph-level insights:
//! - God nodes (highest-degree real entities)
//! - Surprising connections (cross-file or cross-community edges)
//! - Suggested questions (LLM prompts derived from graph structure)
//! - Graph diff (what changed between two snapshots)

use graphify_build::Graph;
use graphify_detect::{FileType, classify_file};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::path::Path;

// ── Language family table ─────────────────────────────────────────────────────

/// Extension → language family, for cross-language suppression logic.
/// Mirrors Python's `_LANG_FAMILY`.
static LANG_FAMILY: std::sync::LazyLock<IndexMap<&'static str, &'static str>> =
    std::sync::LazyLock::new(|| {
        let mut m = IndexMap::new();
        for ext in &[".py", ".pyw"] {
            m.insert(*ext, "python");
        }
        for ext in &[
            ".js", ".jsx", ".mjs", ".ejs", ".ts", ".tsx", ".vue", ".svelte",
        ] {
            m.insert(*ext, "js");
        }
        m.insert(".go", "go");
        m.insert(".rs", "rust");
        for ext in &[".java", ".kt", ".kts", ".scala"] {
            m.insert(*ext, "jvm");
        }
        for ext in &[".c", ".h", ".cpp", ".cc", ".cxx", ".hpp"] {
            m.insert(*ext, "c");
        }
        m.insert(".rb", "ruby");
        m.insert(".swift", "swift");
        m.insert(".cs", "dotnet");
        m.insert(".php", "php");
        m.insert(".r", "r");
        m
    });

/// JSON key labels that indicate a noise node extracted from a JSON schema.
static JSON_NOISE_LABELS: std::sync::LazyLock<IndexSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "start",
            "end",
            "name",
            "id",
            "type",
            "properties",
            "value",
            "key",
            "data",
            "items",
            "title",
            "description",
            "version",
            "dependencies",
            "devdependencies",
            "peerdependencies",
            "optionaldependencies",
            "bundleddependencies",
            "bundledependencies",
        ]
        .into_iter()
        .collect()
    });

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Return true if the two source files belong to different language families.
fn cross_language(src_a: &str, src_b: &str) -> bool {
    let ext_a = Path::new(src_a)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()));
    let ext_b = Path::new(src_b)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()));
    match (ext_a, ext_b) {
        (Some(a), Some(b)) => {
            let fam_a = LANG_FAMILY.get(a.as_str());
            let fam_b = LANG_FAMILY.get(b.as_str());
            matches!((fam_a, fam_b), (Some(a), Some(b)) if a != b)
        }
        _ => false,
    }
}

/// Build a `node_id → community_id` inversion of the communities map.
fn node_community_map(communities: &IndexMap<i64, Vec<String>>) -> IndexMap<String, i64> {
    let mut m = IndexMap::new();
    for (cid, nodes) in communities {
        for n in nodes {
            m.insert(n.clone(), *cid);
        }
    }
    m
}

/// Compute degree (count of incident edges) for every node.
///
/// Mirrors `dict(G.degree())` for undirected graphs.
fn all_degrees(graph: &Graph) -> IndexMap<String, usize> {
    let mut deg: IndexMap<String, usize> = IndexMap::new();
    // Initialise every node at 0
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

/// Return the neighbours of `node_id`.
fn neighbors<'a>(graph: &'a Graph, node_id: &str) -> Vec<&'a str> {
    let directed = graph.kind.is_directed();
    let mut out: Vec<&str> = Vec::new();
    for edge in graph.edges() {
        if edge.source == node_id {
            out.push(&edge.target);
        } else if !directed && edge.target == node_id {
            out.push(&edge.source);
        }
    }
    out
}

/// Return true if a node is a file-level hub or AST method stub.
///
/// Mirrors Python `_is_file_node`.
fn is_file_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return false;
    };
    let label = attrs.get("label").and_then(Value::as_str).unwrap_or("");
    if label.is_empty() {
        return false;
    }
    // File-level hub: label matches the actual source filename
    let source_file = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !source_file.is_empty() {
        let file_name = Path::new(source_file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if label == file_name {
            return true;
        }
    }
    // Method stub: ".method_name()"
    if label.starts_with('.') && label.ends_with("()") {
        return true;
    }
    // Module-level function stub: "function_name()" with degree <= 1
    if label.ends_with("()") {
        let deg = all_degrees(graph);
        if deg.get(node_id).copied().unwrap_or(0) <= 1 {
            return true;
        }
    }
    false
}

/// Return true if the node is a manually-injected semantic concept node.
///
/// Signals: empty `source_file`, or `source_file` has no extension.
///
/// Mirrors Python `_is_concept_node`.
#[must_use]
pub fn is_concept_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return true;
    };
    let source = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("");
    if source.is_empty() {
        return true;
    }
    // No file extension in the last path component
    let last = source.rsplit('/').next().unwrap_or(source);
    !last.contains('.')
}

/// Classify a file path as "code", "paper", "image", or "doc".
///
/// Uses graphify-detect's `classify_file` (same extension list as Python).
///
/// Mirrors Python `_file_category`.
#[must_use]
pub fn file_category(path: &str) -> &'static str {
    match classify_file(Path::new(path)) {
        Some(FileType::Code) => "code",
        Some(FileType::Paper) => "paper",
        Some(FileType::Image) => "image",
        _ => "doc",
    }
}

/// Return true if this is a noise JSON key node that should be excluded.
///
/// Mirrors Python `_is_json_key_node`.
#[must_use]
pub fn is_json_key_node(graph: &Graph, node_id: &str) -> bool {
    let Some(attrs) = graph.node_data(node_id) else {
        return false;
    };
    let src = attrs
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if !std::path::Path::new(&src)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return false;
    }
    let label = attrs
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    JSON_NOISE_LABELS.contains(label.as_str())
}

/// Return the first path component (for cross-repo detection).
fn top_level_dir(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

// ── Betweenness centrality ────────────────────────────────────────────────────

/// Compute approximate or exact betweenness centrality (Brandes' algorithm).
///
/// When `k` is `Some(k)`, uses `k` random pivot nodes (sampled in insertion
/// order, no actual randomness needed for determinism, we take the first k).
/// Mirrors Python `nx.betweenness_centrality(G, k=k, seed=42)`.
///
/// Returns `node_id → centrality` (normalised by `1 / ((n-1)(n-2)/2)` for
/// undirected graphs).
#[allow(clippy::cast_precision_loss)] // graph node counts fit well within f64 mantissa in practice
fn betweenness_centrality(graph: &Graph, k: Option<usize>) -> IndexMap<String, f64> {
    let nodes: Vec<&String> = graph.node_map.keys().collect();
    let n = nodes.len();
    let mut betweenness: IndexMap<String, f64> =
        nodes.iter().map(|&id| (id.clone(), 0.0_f64)).collect();

    if n < 2 {
        return betweenness;
    }

    // Build adjacency for quick lookup
    let directed = graph.kind.is_directed();

    // Index nodes
    let node_idx: IndexMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, &id)| (id.as_str(), i))
        .collect();

    let pivot_count = k.unwrap_or(n).min(n);

    // For each source, run BFS and accumulate pair-dependency
    for s_idx in 0..pivot_count {
        let s = nodes[s_idx].as_str();

        let mut stack: Vec<usize> = Vec::new();
        let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut sigma: Vec<f64> = vec![0.0; n];
        let mut dist: Vec<i64> = vec![-1; n];

        let s_i = node_idx[s];
        sigma[s_i] = 1.0;
        dist[s_i] = 0;

        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        queue.push_back(s_i);

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let v_id = nodes[v].as_str();
            let nbrs = build_neighbor_indices(graph, v_id, &node_idx, directed);
            for w in nbrs {
                if dist[w] < 0 {
                    queue.push_back(w);
                    dist[w] = dist[v] + 1;
                }
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }

        let mut delta: Vec<f64> = vec![0.0; n];
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                if sigma[w] > 0.0 {
                    delta[v] += (sigma[v] / sigma[w]) * (1.0 + delta[w]);
                }
            }
            if w != s_i {
                let id = nodes[w].clone();
                *betweenness.entry(id).or_insert(0.0) += delta[w];
            }
        }
    }

    // Normalise
    let scale = if n > 2 {
        let factor = if directed {
            1.0 / ((n - 1) as f64 * (n - 2) as f64)
        } else {
            2.0 / ((n - 1) as f64 * (n - 2) as f64)
        };
        if k.is_some() {
            // Rescale for sampling (multiply by n/k)
            factor * (n as f64 / pivot_count as f64)
        } else {
            factor
        }
    } else {
        1.0
    };

    for v in betweenness.values_mut() {
        *v *= scale;
    }

    betweenness
}

/// Build list of neighbour indices for betweenness BFS.
fn build_neighbor_indices(
    graph: &Graph,
    node_id: &str,
    node_idx: &IndexMap<&str, usize>,
    directed: bool,
) -> Vec<usize> {
    let mut out = Vec::new();
    for edge in graph.edges() {
        if edge.source == node_id {
            if let Some(&i) = node_idx.get(edge.target.as_str()) {
                out.push(i);
            }
        } else if !directed
            && edge.target == node_id
            && let Some(&i) = node_idx.get(edge.source.as_str())
        {
            out.push(i);
        }
    }
    out
}

/// Compute edge betweenness centrality.
///
/// Mirrors Python `nx.edge_betweenness_centrality(G)`.
#[allow(clippy::cast_precision_loss)] // graph node counts fit well within f64 mantissa in practice
fn edge_betweenness_centrality(graph: &Graph) -> Vec<((String, String), f64)> {
    let nodes: Vec<&String> = graph.node_map.keys().collect();
    let n = nodes.len();

    if n < 2 {
        return Vec::new();
    }

    let directed = graph.kind.is_directed();
    let node_idx: IndexMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, &id)| (id.as_str(), i))
        .collect();

    // Map edge pair → betweenness accumulator
    let mut edge_bet: IndexMap<(usize, usize), f64> = IndexMap::new();
    // Initialise all edges
    for edge in graph.edges() {
        if let (Some(&u), Some(&v)) = (
            node_idx.get(edge.source.as_str()),
            node_idx.get(edge.target.as_str()),
        ) {
            let key = if directed || u < v { (u, v) } else { (v, u) };
            edge_bet.entry(key).or_insert(0.0);
        }
    }

    for s_idx in 0..n {
        let s = nodes[s_idx].as_str();
        let s_i = node_idx[s];

        let mut stack: Vec<usize> = Vec::new();
        let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut sigma: Vec<f64> = vec![0.0; n];
        let mut dist: Vec<i64> = vec![-1; n];

        sigma[s_i] = 1.0;
        dist[s_i] = 0;

        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        queue.push_back(s_i);

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let v_id = nodes[v].as_str();
            let nbrs = build_neighbor_indices(graph, v_id, &node_idx, directed);
            for w in nbrs {
                if dist[w] < 0 {
                    queue.push_back(w);
                    dist[w] = dist[v] + 1;
                }
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }

        let mut delta: Vec<f64> = vec![0.0; n];
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                if sigma[w] > 0.0 {
                    let contribution = (sigma[v] / sigma[w]) * (1.0 + delta[w]);
                    delta[v] += contribution;
                    if w != s_i {
                        let key = if directed || v < w { (v, w) } else { (w, v) };
                        *edge_bet.entry(key).or_insert(0.0) += contribution;
                    }
                }
            }
        }
    }

    // Normalise and convert back to string keys
    let scale = if n > 1 {
        if directed {
            1.0 / ((n - 1) as f64 * n as f64)
        } else {
            2.0 / ((n - 1) as f64 * n as f64)
        }
    } else {
        1.0
    };

    let idx_to_node: Vec<&String> = nodes.clone();
    edge_bet
        .into_iter()
        .map(|((u, v), b)| ((idx_to_node[u].clone(), idx_to_node[v].clone()), b * scale))
        .collect()
}

/// Compute cohesion score for a community.
///
/// Ratio of actual intra-community edges to maximum possible.
/// Inlined from `graphify-cluster/src/cohesion.rs` since the cluster crate
/// is still a stub and doesn't expose this function.
#[allow(clippy::cast_precision_loss)] // community sizes fit well within f64 mantissa in practice
fn cohesion_score(graph: &Graph, community_nodes: &[String]) -> f64 {
    let n = community_nodes.len();
    if n <= 1 {
        return 1.0;
    }
    let node_set: IndexSet<&str> = community_nodes.iter().map(String::as_str).collect();
    let directed = graph.kind.is_directed();
    let mut actual: usize = 0;
    if directed {
        use std::collections::HashSet;
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for edge in graph.edges() {
            if node_set.contains(edge.source.as_str()) && node_set.contains(edge.target.as_str()) {
                let key = if edge.source <= edge.target {
                    (edge.source.clone(), edge.target.clone())
                } else {
                    (edge.target.clone(), edge.source.clone())
                };
                if seen.insert(key) {
                    actual += 1;
                }
            }
        }
    } else {
        for edge in graph.edges() {
            if node_set.contains(edge.source.as_str()) && node_set.contains(edge.target.as_str()) {
                actual += 1;
            }
        }
    }
    let possible = (n * (n - 1)) as f64 / 2.0;
    if possible > 0.0 {
        actual as f64 / possible
    } else {
        0.0
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Return the top-`top_n` most-connected real entities (god nodes).
///
/// File-level hub nodes, concept nodes, and JSON key noise nodes are excluded.
///
/// Mirrors Python `god_nodes`.
#[must_use]
pub fn god_nodes(graph: &Graph, top_n: usize) -> Vec<Value> {
    let degrees = all_degrees(graph);
    let mut sorted: Vec<(&String, usize)> = degrees.iter().map(|(id, &d)| (id, d)).collect();
    sorted.sort_by_key(|item| Reverse(item.1));

    let mut result = Vec::new();
    for (node_id, deg) in sorted {
        if is_file_node(graph, node_id)
            || is_concept_node(graph, node_id)
            || is_json_key_node(graph, node_id)
        {
            continue;
        }
        let label = graph
            .node_data(node_id)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .unwrap_or(node_id);
        result.push(json!({
            "id": node_id,
            "label": label,
            "degree": deg,
        }));
        if result.len() >= top_n {
            break;
        }
    }
    result
}

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
fn cross_file_surprises(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    top_n: usize,
) -> Vec<Value> {
    let node_community = node_community_map(communities);
    let degrees = all_degrees(graph);
    let mut candidates: Vec<(i32, Value)> = Vec::new();

    let structural_relations: IndexSet<&str> = ["imports", "imports_from", "contains", "method"]
        .into_iter()
        .collect();

    for edge in graph.edges() {
        let u = edge.source.as_str();
        let v = edge.target.as_str();
        let data = &edge.attrs;

        let relation = data.get("relation").and_then(Value::as_str).unwrap_or("");
        if structural_relations.contains(relation) {
            continue;
        }
        if is_concept_node(graph, u) || is_concept_node(graph, v) {
            continue;
        }
        if is_file_node(graph, u) || is_file_node(graph, v) {
            continue;
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
            continue;
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

        candidates.push((
            score,
            json!({
                "source": src_label,
                "target": tgt_label,
                "source_files": [src_file, tgt_file],
                "confidence": data.get("confidence").and_then(Value::as_str).unwrap_or("EXTRACTED"),
                "relation": relation,
                "why": why,
            }),
        ));
    }

    candidates.sort_by_key(|item| Reverse(item.0));
    let result: Vec<Value> = candidates.into_iter().map(|(_, v)| v).collect();

    if result.is_empty() {
        return cross_community_surprises(graph, communities, top_n);
    }
    result.into_iter().take(top_n).collect()
}

/// Find surprising connections for single-source corpora.
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

    let mut surprises: Vec<(i32, (i64, i64), Value)> = Vec::new();

    for edge in graph.edges() {
        let u = edge.source.as_str();
        let v = edge.target.as_str();
        let data = &edge.attrs;

        let cid_u = node_community.get(u).copied();
        let cid_v = node_community.get(v).copied();
        let (Some(cu), Some(cv)) = (cid_u, cid_v) else {
            continue;
        };
        if cu == cv {
            continue;
        }
        if is_file_node(graph, u) || is_file_node(graph, v) {
            continue;
        }
        let relation = data.get("relation").and_then(Value::as_str).unwrap_or("");
        if structural_relations.contains(relation) {
            continue;
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
        surprises.push((
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
        ));
    }

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
                if !is_file_node(graph, node_id) && !is_concept_node(graph, node_id) && sc > 0.0 {
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

    // 3. God nodes with many INFERRED edges → verification questions
    let degrees = all_degrees(graph);
    let mut top_nodes: Vec<(&str, usize)> = degrees
        .iter()
        .filter_map(|(id, &d)| {
            if is_file_node(graph, id) {
                None
            } else {
                Some((id.as_str(), d))
            }
        })
        .collect();
    top_nodes.sort_by_key(|item| Reverse(item.1));
    top_nodes.truncate(5);

    for (node_id, _) in top_nodes {
        let inferred: Vec<(&str, &str, &IndexMap<String, Value>)> = graph
            .edges()
            .filter(|e| {
                (e.source == node_id || e.target == node_id)
                    && e.attrs.get("confidence").and_then(Value::as_str) == Some("INFERRED")
            })
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

    // 4. Isolated or weakly-connected nodes → exploration questions
    let deg_map = all_degrees(graph);
    let isolated: Vec<&str> = graph
        .nodes()
        .filter_map(|(id, _)| {
            if deg_map.get(id).copied().unwrap_or(0) <= 1
                && !is_file_node(graph, id)
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

    // 5. Low-cohesion communities → structural questions
    for (cid, nodes) in communities {
        let score = cohesion_score(graph, nodes);
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

    if questions.is_empty() {
        return vec![json!({
            "type": "no_signal",
            "question": null,
            "why": "Not enough signal to generate questions. This usually means the corpus has no AMBIGUOUS edges, no bridge nodes, no INFERRED relationships, and all communities are tightly cohesive. Add more files or run with --mode deep to extract richer edges.",
        })];
    }

    questions.into_iter().take(top_n).collect()
}

/// Compare two graph snapshots and return what changed.
///
/// Returns a JSON object with `new_nodes`, `removed_nodes`, `new_edges`,
/// `removed_edges`, and a `summary` string.
///
/// Mirrors Python `graph_diff`.
#[must_use]
#[allow(clippy::too_many_lines)] // diff collects four lists independently; splitting adds no clarity
pub fn graph_diff(graph_old: &Graph, graph_new: &Graph) -> Value {
    let old_node_ids: IndexSet<&str> = graph_old.nodes().map(|(id, _)| id.as_str()).collect();
    let new_node_ids: IndexSet<&str> = graph_new.nodes().map(|(id, _)| id.as_str()).collect();

    let added_ids: Vec<&str> = new_node_ids
        .iter()
        .filter(|id| !old_node_ids.contains(*id))
        .copied()
        .collect();
    let removed_ids: Vec<&str> = old_node_ids
        .iter()
        .filter(|id| !new_node_ids.contains(*id))
        .copied()
        .collect();

    let new_nodes: Vec<Value> = added_ids
        .iter()
        .map(|&id| {
            let label = graph_new
                .node_data(id)
                .and_then(|a| a.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(id);
            json!({"id": id, "label": label})
        })
        .collect();

    let removed_nodes: Vec<Value> = removed_ids
        .iter()
        .map(|&id| {
            let label = graph_old
                .node_data(id)
                .and_then(|a| a.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(id);
            json!({"id": id, "label": label})
        })
        .collect();

    // Edge key function: (min(u,v), max(u,v), relation) for undirected
    let directed = graph_old.kind.is_directed() || graph_new.kind.is_directed();
    let edge_key = |u: &str, v: &str, relation: &str| -> (String, String, String) {
        if directed {
            (u.to_string(), v.to_string(), relation.to_string())
        } else {
            let (a, b) = if u <= v { (u, v) } else { (v, u) };
            (a.to_string(), b.to_string(), relation.to_string())
        }
    };

    let old_edge_keys: IndexSet<(String, String, String)> = graph_old
        .edges()
        .map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            edge_key(&e.source, &e.target, rel)
        })
        .collect();

    let new_edge_keys: IndexSet<(String, String, String)> = graph_new
        .edges()
        .map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            edge_key(&e.source, &e.target, rel)
        })
        .collect();

    let new_edges: Vec<Value> = graph_new
        .edges()
        .filter_map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = edge_key(&e.source, &e.target, rel);
            if old_edge_keys.contains(&key) {
                None
            } else {
                let conf = e
                    .attrs
                    .get("confidence")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Some(json!({
                    "source": e.source,
                    "target": e.target,
                    "relation": rel,
                    "confidence": conf,
                }))
            }
        })
        .collect();

    let removed_edges: Vec<Value> = graph_old
        .edges()
        .filter_map(|e| {
            let rel = e
                .attrs
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = edge_key(&e.source, &e.target, rel);
            if new_edge_keys.contains(&key) {
                None
            } else {
                let conf = e
                    .attrs
                    .get("confidence")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Some(json!({
                    "source": e.source,
                    "target": e.target,
                    "relation": rel,
                    "confidence": conf,
                }))
            }
        })
        .collect();

    // Build summary
    let mut parts: Vec<String> = Vec::new();
    let nn = new_nodes.len();
    let ne = new_edges.len();
    let rn = removed_nodes.len();
    let re = removed_edges.len();
    if nn > 0 {
        parts.push(format!("{nn} new node{}", if nn == 1 { "" } else { "s" }));
    }
    if ne > 0 {
        parts.push(format!("{ne} new edge{}", if ne == 1 { "" } else { "s" }));
    }
    if rn > 0 {
        parts.push(format!(
            "{rn} node{} removed",
            if rn == 1 { "" } else { "s" }
        ));
    }
    if re > 0 {
        parts.push(format!(
            "{re} edge{} removed",
            if re == 1 { "" } else { "s" }
        ));
    }
    let summary = if parts.is_empty() {
        "no changes".to_string()
    } else {
        parts.join(", ")
    };

    json!({
        "new_nodes": new_nodes,
        "removed_nodes": removed_nodes,
        "new_edges": new_edges,
        "removed_edges": removed_edges,
        "summary": summary,
    })
}
