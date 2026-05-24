//! Graph-query helpers — pure functions that work on [`Graph`].
//!
//! Ports all `_`-prefixed helper functions from `graphify-py/graphify/serve.py`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::BuildHasher;
use std::path::Path;

use graphify_build::Graph;
use indexmap::IndexMap;
use serde_json::Value;

use crate::ServeError;

// ── Constants ─────────────────────────────────────────────────────────────────

const EXACT_MATCH_BONUS: f64 = 1000.0;
const PREFIX_MATCH_BONUS: f64 = 100.0;
const SUBSTRING_MATCH_BONUS: f64 = 1.0;
const SOURCE_MATCH_BONUS: f64 = 0.5;

// ── Unicode helpers ───────────────────────────────────────────────────────────

/// Remove combining diacritical marks (NFKD decompose then strip combining chars).
///
/// Mirrors Python `_strip_diacritics`.
#[must_use]
pub fn strip_diacritics(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}

// ── Graph loading ─────────────────────────────────────────────────────────────

/// Load a graph from a JSON file.
///
/// Mirrors Python `_load_graph`. Exits via `Err(ServeError)` instead of
/// `sys.exit(1)` so the server can surface the error cleanly.
///
/// # Errors
///
/// Returns [`ServeError`] if the file is missing, not a `.json` extension,
/// cannot be read, or contains invalid JSON.
pub fn load_graph(graph_path: &str) -> Result<Graph, ServeError> {
    let p = Path::new(graph_path);
    // Canonicalize if possible; fall back to raw path for non-existent paths.
    let resolved = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    if resolved.extension().and_then(|e| e.to_str()) != Some("json") {
        return Err(ServeError::InvalidPath(format!(
            "Graph path must be a .json file, got: {graph_path:?}"
        )));
    }
    if !resolved.exists() {
        return Err(ServeError::NotFound(format!(
            "Graph file not found: {}",
            resolved.display()
        )));
    }

    graphify_security::check_graph_file_size_cap(&resolved)
        .map_err(|e| ServeError::Io(format!("{e}")))?;

    let text = std::fs::read_to_string(&resolved).map_err(|e| ServeError::Io(format!("{e}")))?;

    let mut data: Value =
        serde_json::from_str(&text).map_err(|e| ServeError::CorruptedGraph(format!("{e}")))?;

    // Python: if "links" not in data and "edges" in data → rename edges→links
    // then force directed=True.
    if let Some(obj) = data.as_object_mut() {
        if !obj.contains_key("links") && obj.contains_key("edges") {
            let edges = obj.remove("edges");
            if let Some(e) = edges {
                obj.insert("links".to_string(), e);
            }
        }
        obj.insert("directed".to_string(), Value::Bool(true));
    }

    graphify_build::build_from_json(data, true, None).map_err(|e| ServeError::Io(format!("{e}")))
}

// ── Communities ───────────────────────────────────────────────────────────────

/// Reconstruct community map from node attributes.
///
/// Mirrors Python `_communities_from_graph`.
#[must_use]
pub fn communities_from_graph(graph: &Graph) -> IndexMap<i64, Vec<String>> {
    let mut communities: IndexMap<i64, Vec<String>> = IndexMap::new();
    for (node_id, attrs) in graph.nodes() {
        if let Some(cid) = attrs.get("community").and_then(Value::as_i64) {
            communities.entry(cid).or_default().push(node_id.clone());
        }
    }
    communities
}

// ── IDF weighting ─────────────────────────────────────────────────────────────

/// Compute IDF weights for query terms.
///
/// Results are stored in `idf_cache` and returned. The cache is keyed on the
/// term so repeated queries don't recompute.
///
/// Mirrors Python `_compute_idf`.
#[must_use]
#[allow(clippy::cast_precision_loss)] // graph node count fits comfortably in f64.
pub fn compute_idf<'a, S: BuildHasher>(
    graph: &Graph,
    terms: &[&'a str],
    idf_cache: &mut HashMap<String, f64, S>,
) -> HashMap<&'a str, f64> {
    let n = graph.node_count().max(1) as f64;
    let uncached: Vec<&str> = terms
        .iter()
        .copied()
        .filter(|t| !idf_cache.contains_key(*t))
        .collect();

    if !uncached.is_empty() {
        let mut df: HashMap<&str, usize> = uncached.iter().map(|t| (*t, 0_usize)).collect();
        for (_, attrs) in graph.nodes() {
            let norm_label = get_norm_label(attrs);
            for t in &uncached {
                if norm_label.contains(*t) {
                    *df.entry(t).or_default() += 1;
                }
            }
        }
        for t in &uncached {
            #[allow(clippy::cast_precision_loss)] // Document frequency cast; acceptable.
            let d = *df.get(t).unwrap_or(&0) as f64;
            idf_cache.insert((*t).to_string(), (1.0 + n / (1.0 + d)).ln());
        }
    }

    terms
        .iter()
        .map(|t| (*t, *idf_cache.get(*t).unwrap_or(&(1.0 + n).ln())))
        .collect()
}

/// Return the pre-computed normalised label for a node, falling back to a
/// diacritic-stripped lowercase version of the raw `label` attribute.
fn get_norm_label(attrs: &IndexMap<String, Value>) -> String {
    if let Some(Value::String(s)) = attrs.get("norm_label")
        && !s.is_empty()
    {
        return s.clone();
    }
    let label = attrs.get("label").and_then(Value::as_str).unwrap_or("");
    strip_diacritics(label).to_lowercase()
}

// ── Node scoring ─────────────────────────────────────────────────────────────

/// Score nodes against query terms using IDF-weighted fuzzy matching.
///
/// Returns `(score, node_id)` pairs sorted highest-score first.
///
/// Mirrors Python `_score_nodes`.
#[must_use]
#[allow(clippy::cast_precision_loss)] // graph node count fits comfortably in f64.
pub fn score_nodes<S: BuildHasher>(
    graph: &Graph,
    terms: &[&str],
    idf_cache: &mut HashMap<String, f64, S>,
) -> Vec<(f64, String)> {
    let norm_terms: Vec<String> = terms
        .iter()
        .map(|t| strip_diacritics(t).to_lowercase())
        .collect();
    let norm_term_refs: Vec<&str> = norm_terms.iter().map(String::as_str).collect();
    let idf = compute_idf(graph, &norm_term_refs, idf_cache);

    let mut scored: Vec<(f64, String)> = Vec::new();
    for (nid, attrs) in graph.nodes() {
        let norm_label = get_norm_label(attrs);
        let bare_label = norm_label.trim_end_matches(['(', ')']).to_string();
        let source = attrs
            .get("source_file")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();

        let mut score = 0.0_f64;
        for t in &norm_terms {
            let w = idf.get(t.as_str()).copied().unwrap_or(1.0);
            // Three-tier: exact > prefix > substring (take strongest per term).
            if t == &norm_label || t == &bare_label {
                score += EXACT_MATCH_BONUS * w;
            } else if norm_label.starts_with(t.as_str()) || bare_label.starts_with(t.as_str()) {
                score += PREFIX_MATCH_BONUS * w;
            } else if norm_label.contains(t.as_str()) {
                score += SUBSTRING_MATCH_BONUS * w;
            }
            if source.contains(t.as_str()) {
                score += SOURCE_MATCH_BONUS * w;
            }
        }
        if score > 0.0 {
            scored.push((score, nid.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

// ── Seed selection ────────────────────────────────────────────────────────────

/// Select BFS seed nodes, stopping when score drops too far below the top.
///
/// Mirrors Python `_pick_seeds`.
#[must_use]
pub fn pick_seeds(scored: &[(f64, String)], max_k: usize, gap_ratio: f64) -> Vec<String> {
    if scored.is_empty() {
        return Vec::new();
    }
    let top_score = scored[0].0;
    let mut seeds = Vec::new();
    for (score, nid) in scored.iter().take(max_k) {
        if !seeds.is_empty() && *score < top_score * gap_ratio {
            break;
        }
        seeds.push(nid.clone());
    }
    seeds
}

// ── Context filters ───────────────────────────────────────────────────────────

const CONTEXT_HINTS: &[(&str, &[&str])] = &[
    (
        "call",
        &["call", "calls", "called", "invoke", "invokes", "invoked"],
    ),
    (
        "import",
        &["import", "imports", "imported", "module", "modules"],
    ),
    (
        "field",
        &[
            "field",
            "fields",
            "member",
            "members",
            "property",
            "properties",
        ],
    ),
    (
        "parameter_type",
        &[
            "parameter",
            "parameters",
            "param",
            "params",
            "argument",
            "arguments",
        ],
    ),
    ("return_type", &["return", "returns", "returned"]),
    (
        "generic_arg",
        &["generic", "generics", "template", "templates"],
    ),
];

/// Normalise an explicit filter list (deduplicate, strip whitespace).
///
/// Mirrors Python `_normalize_context_filters`.
#[must_use]
pub fn normalize_context_filters(filters: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for value in filters {
        let key = strip_diacritics(value.trim()).to_lowercase();
        if !key.is_empty() && seen.insert(key.clone()) {
            normalized.push(key);
        }
    }
    normalized
}

/// Infer context filters from question text.
///
/// Mirrors Python `_infer_context_filters`.
#[must_use]
pub fn infer_context_filters(question: &str) -> Vec<String> {
    let lowered: HashSet<String> = question
        .replace(['?', ','], " ")
        .split_whitespace()
        .map(|t| strip_diacritics(t).to_lowercase())
        .collect();
    let mut inferred: Vec<String> = Vec::new();
    for (context, hints) in CONTEXT_HINTS {
        if hints.iter().any(|h| lowered.contains(*h)) {
            inferred.push((*context).to_string());
        }
    }
    inferred
}

/// Resolve context filters: explicit wins over heuristic inference.
///
/// Returns `(filters, source)` where source is `"explicit"`, `"heuristic"`, or `None`.
///
/// Mirrors Python `_resolve_context_filters`.
#[must_use]
pub fn resolve_context_filters(
    question: &str,
    explicit: Option<&[String]>,
) -> (Vec<String>, Option<String>) {
    let normalized = explicit.map_or_else(Vec::new, normalize_context_filters);
    if !normalized.is_empty() {
        return (normalized, Some("explicit".to_string()));
    }
    let inferred = infer_context_filters(question);
    if !inferred.is_empty() {
        return (inferred, Some("heuristic".to_string()));
    }
    (Vec::new(), None)
}

// ── Context-filtered graph view ───────────────────────────────────────────────

/// Build a filtered graph keeping only edges whose `context` is in `filters`.
///
/// Mirrors Python `_filter_graph_by_context`.
#[must_use]
pub fn filter_graph_by_context(graph: &Graph, context_filters: Option<&[String]>) -> Graph {
    let filters: HashSet<String> = context_filters
        .map(|f| normalize_context_filters(f).into_iter().collect())
        .unwrap_or_default();

    if filters.is_empty() {
        return graph.clone();
    }

    let mut h = Graph::new(graph.kind);
    for (id, attrs) in graph.nodes() {
        h.add_node(id, attrs.clone());
    }
    for edge in graph.edges() {
        let ctx = edge.attrs.get("context").and_then(Value::as_str);
        if ctx.is_some_and(|c| filters.contains(c)) {
            h.add_edge(&edge.source, &edge.target, edge.attrs.clone());
        }
    }
    h
}

// ── Hub threshold ─────────────────────────────────────────────────────────────

/// Compute hub threshold (p99 of degree distribution, floored at 50).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
// p99 index: float multiply then cast back to index; precision loss is harmless.
fn hub_threshold(graph: &Graph) -> usize {
    let mut degrees: Vec<usize> = graph
        .nodes()
        .map(|(id, _)| node_degree(graph, id))
        .collect();
    if degrees.is_empty() {
        return 50;
    }
    degrees.sort_unstable();
    let p99_idx = (degrees.len() as f64 * 0.99) as usize;
    let idx = p99_idx.min(degrees.len() - 1);
    50_usize.max(degrees[idx])
}

/// Total degree of a node (edges touching this node).
#[must_use]
pub fn node_degree(graph: &Graph, node_id: &str) -> usize {
    graph
        .edges()
        .filter(|e| e.source == node_id || e.target == node_id)
        .count()
}

/// All direct neighbors (successors for directed, adjacent for undirected).
#[must_use]
pub fn neighbors(graph: &Graph, node_id: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if graph.kind.is_directed() {
        for edge in graph.edges() {
            if edge.source == node_id {
                out.push(edge.target.clone());
            }
        }
    } else {
        for edge in graph.edges() {
            if edge.source == node_id {
                out.push(edge.target.clone());
            } else if edge.target == node_id {
                out.push(edge.source.clone());
            }
        }
    }
    out
}

/// Successors (directed out-edges; for undirected identical to neighbors).
#[must_use]
pub fn successors(graph: &Graph, node_id: &str) -> Vec<String> {
    graph
        .edges()
        .filter_map(|e| {
            if e.source == node_id {
                Some(e.target.clone())
            } else if !graph.kind.is_directed() && e.target == node_id {
                Some(e.source.clone())
            } else {
                None
            }
        })
        .collect()
}

/// Predecessors (directed in-edges).
#[must_use]
pub fn predecessors(graph: &Graph, node_id: &str) -> Vec<String> {
    graph
        .edges()
        .filter_map(|e| {
            if e.target == node_id {
                Some(e.source.clone())
            } else {
                None
            }
        })
        .collect()
}

// ── BFS / DFS traversal ───────────────────────────────────────────────────────

/// BFS from `start_nodes` up to `depth` hops.
///
/// Returns `(visited_set, edges_seen)`.
///
/// Mirrors Python `_bfs`.
#[must_use]
pub fn bfs(
    graph: &Graph,
    start_nodes: &[String],
    depth: usize,
) -> (HashSet<String>, Vec<(String, String)>) {
    let hub = hub_threshold(graph);
    let seed_set: HashSet<&str> = start_nodes.iter().map(String::as_str).collect();
    let mut visited: HashSet<String> = start_nodes.iter().cloned().collect();
    let mut frontier: HashSet<String> = start_nodes.iter().cloned().collect();
    let mut edges_seen: Vec<(String, String)> = Vec::new();

    for _ in 0..depth {
        let mut next_frontier: HashSet<String> = HashSet::new();
        for n in &frontier {
            // Don't expand through high-degree hubs (except seeds).
            if !seed_set.contains(n.as_str()) && node_degree(graph, n) >= hub {
                continue;
            }
            for neighbor in neighbors(graph, n) {
                if !visited.contains(&neighbor) {
                    next_frontier.insert(neighbor.clone());
                    edges_seen.push((n.clone(), neighbor));
                }
            }
        }
        visited.extend(next_frontier.iter().cloned());
        frontier = next_frontier;
    }
    (visited, edges_seen)
}

/// DFS from `start_nodes` up to `depth` hops.
///
/// Returns `(visited_set, edges_seen)`.
///
/// Mirrors Python `_dfs`.
#[must_use]
pub fn dfs(
    graph: &Graph,
    start_nodes: &[String],
    depth: usize,
) -> (HashSet<String>, Vec<(String, String)>) {
    let hub = hub_threshold(graph);
    let seed_set: HashSet<&str> = start_nodes.iter().map(String::as_str).collect();
    let mut visited: HashSet<String> = HashSet::new();
    let mut edges_seen: Vec<(String, String)> = Vec::new();
    // Stack: (node, depth). Reversed so first start_node is processed first.
    let mut stack: Vec<(String, usize)> =
        start_nodes.iter().rev().map(|n| (n.clone(), 0)).collect();

    while let Some((node, d)) = stack.pop() {
        if visited.contains(&node) || d > depth {
            continue;
        }
        visited.insert(node.clone());
        if !seed_set.contains(node.as_str()) && node_degree(graph, &node) >= hub {
            continue;
        }
        for neighbor in neighbors(graph, &node) {
            if !visited.contains(&neighbor) {
                stack.push((neighbor.clone(), d + 1));
                edges_seen.push((node.clone(), neighbor));
            }
        }
    }
    (visited, edges_seen)
}

// ── Subgraph text rendering ───────────────────────────────────────────────────

/// Render subgraph as text, truncating at `token_budget` (approx 3 chars/token).
///
/// Mirrors Python `_subgraph_to_text`.
#[must_use]
pub fn subgraph_to_text<S: BuildHasher>(
    graph: &Graph,
    nodes: &HashSet<String, S>,
    edges: &[(String, String)],
    token_budget: usize,
    seeds: Option<&[String]>,
) -> String {
    use graphify_security::sanitize_label;

    let char_budget = token_budget * 3;
    let seed_set: HashSet<&str> =
        seeds.map_or_else(HashSet::new, |s| s.iter().map(String::as_str).collect());

    // Seeds first, then remaining sorted by degree descending.
    let mut ordered: Vec<&String> = seeds.map_or_else(Vec::new, |s| {
        s.iter().filter(|n| nodes.contains(*n)).collect()
    });
    let mut rest: Vec<&String> = nodes
        .iter()
        .filter(|n| !seed_set.contains(n.as_str()))
        .collect();
    rest.sort_by_key(|n| std::cmp::Reverse(node_degree(graph, n)));
    ordered.extend(rest);

    let mut lines: Vec<String> = Vec::new();
    for nid in &ordered {
        let empty = IndexMap::new();
        let d = graph.node_data(nid).unwrap_or(&empty);
        let line = format!(
            "NODE {} [src={} loc={} community={}]",
            sanitize_label(d.get("label").and_then(Value::as_str).or(Some(nid))),
            sanitize_label(d.get("source_file").and_then(Value::as_str).or(Some(""))),
            sanitize_label(
                d.get("source_location")
                    .and_then(Value::as_str)
                    .or(Some(""))
            ),
            sanitize_label(d.get("community").map(ToString::to_string).as_deref()),
        );
        lines.push(line);
    }
    for (u, v) in edges {
        if nodes.contains(u) && nodes.contains(v) {
            let empty = IndexMap::new();
            let d = graph.edge_data(u, v).unwrap_or(&empty);
            let context = d.get("context").and_then(Value::as_str);
            let context_suffix = context.map_or_else(String::new, |c| {
                format!(" context={}", sanitize_label(Some(c)))
            });
            let empty_node = IndexMap::new();
            let u_label = graph
                .node_data(u)
                .unwrap_or(&empty_node)
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(u);
            let v_label = graph
                .node_data(v)
                .unwrap_or(&empty_node)
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(v);
            let line = format!(
                "EDGE {} --{} [{}{}]--> {}",
                sanitize_label(Some(u_label)),
                sanitize_label(d.get("relation").and_then(Value::as_str).or(Some(""))),
                sanitize_label(d.get("confidence").and_then(Value::as_str).or(Some(""))),
                context_suffix,
                sanitize_label(Some(v_label)),
            );
            lines.push(line);
        }
    }

    let output = lines.join("\n");
    if output.len() <= char_budget {
        return output;
    }

    let cut_at = output[..char_budget]
        .rfind('\n')
        .filter(|&p| p > 0)
        .unwrap_or(char_budget);
    let total_nodes = lines.iter().filter(|l| l.starts_with("NODE ")).count();
    let truncated_prefix = &output[..cut_at];
    let shown_nodes = truncated_prefix
        .split('\n')
        .filter(|l| l.starts_with("NODE "))
        .count();
    let cut_count = total_nodes.saturating_sub(shown_nodes);
    format!(
        "{truncated_prefix}\n... (truncated — {cut_count} more nodes cut by ~{token_budget}-token budget.\
 Narrow with context_filter=['call'] or use get_node for a specific symbol)"
    )
}

// ── Find node ─────────────────────────────────────────────────────────────────

/// Return node IDs whose label or ID matches search term (diacritic-insensitive).
///
/// Ordered: exact, prefix, substring.
///
/// Mirrors Python `_find_node`.
#[must_use]
pub fn find_node(graph: &Graph, label: &str) -> Vec<String> {
    let term = strip_diacritics(label).to_lowercase();
    let mut exact: Vec<String> = Vec::new();
    let mut prefix: Vec<String> = Vec::new();
    let mut substring: Vec<String> = Vec::new();

    for (nid, attrs) in graph.nodes() {
        let norm_label = get_norm_label(attrs);
        let bare_label = norm_label.trim_end_matches(['(', ')']).to_string();
        let nid_lower = nid.to_lowercase();
        if term == norm_label || term == bare_label || term == nid_lower {
            exact.push(nid.clone());
        } else if norm_label.starts_with(&term)
            || bare_label.starts_with(&term)
            || nid_lower.starts_with(&term)
        {
            prefix.push(nid.clone());
        } else if norm_label.contains(&term) {
            substring.push(nid.clone());
        }
    }
    exact.extend(prefix);
    exact.extend(substring);
    exact
}

// ── Shortest path ─────────────────────────────────────────────────────────────

/// BFS-based shortest path over an undirected view.
///
/// Returns node IDs along the path (inclusive), or `None` if unreachable.
#[must_use]
pub fn shortest_path(graph: &Graph, src: &str, tgt: &str) -> Option<Vec<String>> {
    if src == tgt {
        return Some(vec![src.to_string()]);
    }
    // BFS treating graph as undirected.
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut came_from: HashMap<String, String> = HashMap::new();
    queue.push_back(src.to_string());
    came_from.insert(src.to_string(), src.to_string());

    while let Some(node) = queue.pop_front() {
        // All adjacent nodes (both directions).
        let adjs: Vec<String> = graph
            .edges()
            .filter_map(|e| {
                if e.source == node {
                    Some(e.target.clone())
                } else if e.target == node {
                    Some(e.source.clone())
                } else {
                    None
                }
            })
            .collect();
        for nb in adjs {
            if !came_from.contains_key(&nb) {
                came_from.insert(nb.clone(), node.clone());
                if nb == tgt {
                    // Reconstruct path.
                    let mut path = vec![nb.clone()];
                    let mut cur = nb.clone();
                    while cur != src {
                        let prev = came_from[&cur].clone();
                        path.push(prev.clone());
                        cur = prev;
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(nb);
            }
        }
    }
    None
}

// ── Main query entry point ────────────────────────────────────────────────────

/// Split a query string into searchable terms.
///
/// Terms are lowercased; short tokens (≤ 2 chars) are dropped only when
/// they are entirely English (ASCII `a-z`). Non-ASCII short tokens such as
/// CJK characters are kept so non-English queries remain searchable (#964).
/// Mirrors Python `_query_terms` in `serve.py`.
#[must_use]
pub fn query_terms(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in question.split_whitespace() {
        let lower = raw.to_lowercase();
        if lower.is_empty() {
            continue;
        }
        let is_english_only = lower.chars().all(|c| c.is_ascii_lowercase());
        if !is_english_only || lower.chars().count() > 2 {
            out.push(lower);
        }
    }
    out
}

/// High-level graph query: search, traverse, and render as text.
///
/// Mirrors Python `_query_graph_text`.
#[must_use]
pub fn query_graph_text<S: BuildHasher>(
    graph: &Graph,
    question: &str,
    mode: &str,
    depth: usize,
    token_budget: usize,
    context_filters: Option<&[String]>,
    idf_cache: &mut HashMap<String, f64, S>,
) -> String {
    let terms: Vec<String> = query_terms(question);
    let term_refs: Vec<&str> = terms.iter().map(String::as_str).collect();
    let scored = score_nodes(graph, &term_refs, idf_cache);
    let start_nodes = pick_seeds(&scored, 3, 0.2);
    if start_nodes.is_empty() {
        return "No matching nodes found.".to_string();
    }

    let (resolved_filters, filter_source) = resolve_context_filters(question, context_filters);
    let filter_opt: Option<&[String]> = if resolved_filters.is_empty() {
        None
    } else {
        Some(&resolved_filters)
    };
    let traversal_graph = filter_graph_by_context(graph, filter_opt);

    let (nodes, edges) = if mode == "dfs" {
        dfs(&traversal_graph, &start_nodes, depth)
    } else {
        bfs(&traversal_graph, &start_nodes, depth)
    };

    let mut header_parts: Vec<String> = vec![
        format!("Traversal: {} depth={}", mode.to_uppercase(), depth),
        format!(
            "Start: {:?}",
            start_nodes
                .iter()
                .map(|n| graph
                    .node_data(n)
                    .and_then(|a| a.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(n))
                .collect::<Vec<_>>()
        ),
    ];
    if !resolved_filters.is_empty()
        && let Some(src) = &filter_source
    {
        header_parts.push(format!("Context: {} ({src})", resolved_filters.join(", ")));
    }
    header_parts.push(format!("{} nodes found", nodes.len()));
    let header = header_parts.join(" | ") + "\n\n";
    header
        + &subgraph_to_text(
            &traversal_graph,
            &nodes,
            &edges,
            token_budget,
            Some(&start_nodes),
        )
}
