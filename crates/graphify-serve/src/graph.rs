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

/// Split `text` into `\w+` runs (Unicode alphanumeric characters plus `_`),
/// matching Python's `re.findall(r"\w+", text)` under its default Unicode mode.
/// `char::is_alphanumeric()` accepts accented letters, CJK, etc. — broader than
/// an ASCII-only `\w`, which is the desired behaviour for international labels.
fn word_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
}

/// Split text into word tokens, stripping punctuation and diacritics.
///
/// Mirrors Python `_search_tokens` (#1037).
#[must_use]
fn search_tokens(text: &str) -> Vec<String> {
    let normalized = strip_diacritics(text).to_lowercase();
    word_tokens(&normalized).map(str::to_string).collect()
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

    // #1504: nudge once when the on-disk graph still uses the pre-path-qualified
    // node-ID scheme, so an MCP session sees the same advice as the CLI. Inspect
    // the raw nodes before `build_from_json` moves `data`; silent on fresh graphs.
    if let Some(nodes) = data.get("nodes").and_then(Value::as_array)
        && graphify_build::graph_has_legacy_ids(nodes, None)
    {
        eprintln!(
            "[graphify] note: this graph uses the pre-#1504 node-ID scheme; \
             rebuild with `graphify extract --force` for path-qualified IDs."
        );
    }

    let mut graph = graphify_build::build_from_json(data, true, None)
        .map_err(|e| ServeError::Io(format!("{e}")))?;
    // Stash the work-memory overlay (if any) so query text can annotate a node
    // with its learned status (#1441). Best-effort — an absent/failed load leaves
    // it empty; graph.json itself stays purely structural.
    let overlay: serde_json::Map<String, Value> =
        graphify_reflect::load_learning_overlay(&resolved)
            .into_iter()
            .collect();
    graph
        .graph_attrs
        .insert("_learning_overlay".to_string(), Value::Object(overlay));
    Ok(graph)
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
    // Dedupe tokens order-preserving (as pick_seeds does): a repeated query word
    // must not double-count every tier, and with the coverage scaling below it
    // would also inflate the matched-term ratio (#1602).
    let mut seen_terms: std::collections::HashSet<String> = std::collections::HashSet::new();
    let norm_terms: Vec<String> = terms
        .iter()
        .flat_map(|t| search_tokens(t))
        .filter(|tok| seen_terms.insert(tok.clone()))
        .collect();
    let n_terms = norm_terms.len();
    let norm_term_refs: Vec<&str> = norm_terms.iter().map(String::as_str).collect();
    let idf = compute_idf(graph, &norm_term_refs, idf_cache);

    // Whole-query string + its weight (the rarest constituent term's idf;
    // per-term default 1.0). Mirrors Python `_score_nodes`: a multi-word query
    // that equals (or prefixes) the whole label must dominate the per-token
    // bag-of-words sums so `path`/`query` resolve the same node `explain` does.
    let joined = norm_terms.join(" ");
    let joined_w = norm_terms
        .iter()
        .map(|t| idf.get(t.as_str()).copied().unwrap_or(1.0))
        .reduce(f64::max)
        .unwrap_or(1.0);

    let mut scored: Vec<(f64, usize, String)> = Vec::new();
    for (nid, attrs) in graph.nodes() {
        let norm_label = get_norm_label(attrs);
        let bare_label = norm_label.trim_end_matches(['(', ')']).to_string();
        let raw_label = attrs.get("label").and_then(Value::as_str).unwrap_or("");
        let label_tokens = search_tokens(raw_label).join(" ");
        let source = attrs
            .get("source_file")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();

        let mut score = 0.0_f64;
        // Full-query tier: an exact/prefix match of the whole label outranks the
        // per-token sums below (×10), weighted by the rarest term so a specific
        // multi-word label beats common-token noise.
        if !joined.is_empty() {
            let nid_lower = nid.to_lowercase();
            if joined == norm_label
                || joined == bare_label
                || joined == label_tokens
                || joined == nid_lower
            {
                score += EXACT_MATCH_BONUS * 10.0 * joined_w;
            } else if norm_label.starts_with(joined.as_str())
                || bare_label.starts_with(joined.as_str())
                || label_tokens.starts_with(joined.as_str())
            {
                score += PREFIX_MATCH_BONUS * 10.0 * joined_w;
            }
        }
        // Term coverage (#1602): scale the per-term exact/prefix tiers by the
        // squared fraction of query terms the node's LABEL matches, so a lone
        // generic word equal to a short label can't bury nodes matching several
        // terms. Substring hits and source-file hits score directly (unscaled);
        // source hits do NOT count toward coverage. Single-term / full-coverage
        // queries are unchanged (coverage == 1).
        let mut matched = 0_usize;
        let mut tiered = 0.0_f64;
        for t in &norm_terms {
            let w = idf.get(t.as_str()).copied().unwrap_or(1.0);
            // Three-tier: exact > prefix > substring (take strongest per term).
            if t == &norm_label || t == &bare_label {
                tiered += EXACT_MATCH_BONUS * w;
                matched += 1;
            } else if norm_label.starts_with(t.as_str()) || bare_label.starts_with(t.as_str()) {
                tiered += PREFIX_MATCH_BONUS * w;
                matched += 1;
            } else if norm_label.contains(t.as_str()) {
                score += SUBSTRING_MATCH_BONUS * w;
                matched += 1;
            }
            if source.contains(t.as_str()) {
                score += SOURCE_MATCH_BONUS * w;
            }
        }
        if tiered > 0.0 && n_terms > 0 {
            let coverage = matched as f64 / n_terms as f64;
            score += tiered * coverage * coverage;
        }
        if score > 0.0 {
            // Tie-break toward the shorter label, then node id, so a concise
            // exact match beats a longer superset of equal score.
            let label_len = if raw_label.is_empty() {
                nid.chars().count()
            } else {
                raw_label.chars().count()
            };
            scored.push((score, label_len, nid.clone()));
        }
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    scored.into_iter().map(|(s, _, nid)| (s, nid)).collect()
}

/// Pick a path endpoint from a [`score_nodes`] result, preferring full-token
/// matches.
///
/// The full-query tier in `score_nodes` only fires when the query equals or
/// prefixes a label, so a query that is a token *subset* of the intended label
/// (query "Reject-everything judge" vs. label "Degenerate Reject-Everything
/// Judge") gets no bonus, and a node prefix-matching one rare token (label
/// "Rejection Summary") can out-score it on IDF alone — anchoring the path on an
/// unrelated, often disconnected node and yielding a false "No path found".
/// Scan the score-ordered list and take the first candidate whose label contains
/// EVERY query token; when the head already full-matches (or none does) this is
/// exactly `scored[0]`. Mirrors Python `_pick_scored_endpoint`.
#[must_use]
pub fn pick_scored_endpoint(graph: &Graph, scored: &[(f64, String)], query: &str) -> String {
    let head = || scored.first().map_or_else(String::new, |(_, n)| n.clone());
    let qtokens: std::collections::HashSet<String> = search_tokens(query).into_iter().collect();
    if qtokens.is_empty() {
        return head();
    }
    for (_score, nid) in scored {
        let label = graph
            .node_map
            .get(nid)
            .and_then(|a| a.get("label"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(nid);
        let ltokens: std::collections::HashSet<String> = search_tokens(label).into_iter().collect();
        if qtokens.is_subset(&ltokens) {
            return nid.clone();
        }
    }
    head()
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

/// [`pick_seeds`] plus the per-term seed guarantee (#1445).
///
/// After the gap-ratio cutoff, guarantees at least one seed per distinct query
/// term that has any match at all, so one term's incidental exact-match
/// collision cannot starve out the query's other, actually-relevant terms.
/// Ties within a term break by graph degree (structural centrality), so an
/// isolated incidental match doesn't out-rank a well-connected hub for that
/// term. Mirrors Python `_pick_seeds(..., G=..., terms=...)`.
#[must_use]
pub fn pick_seeds_diverse<S: BuildHasher>(
    scored: &[(f64, String)],
    max_k: usize,
    gap_ratio: f64,
    graph: &Graph,
    terms: &[&str],
    idf_cache: &mut HashMap<String, f64, S>,
) -> Vec<String> {
    if scored.is_empty() {
        return Vec::new();
    }
    // Dedup seeds by normalized label so a generic, homonymous symbol — dozens of
    // route handlers all labelled `GET`/`POST`, a `handler` repeated across a
    // framework — contributes at most one seed instead of consuming every slot
    // and flooding the BFS with near-identical neighborhoods (#1766). The key
    // mirrors `get_norm_label` (score_nodes' normalization) so `GET`/`Get`/`get`
    // collapse; a node absent from the graph falls back to its (unique) id.
    let seed_label_key = |nid: &str| -> String {
        graph.node_map.get(nid).map_or_else(
            || nid.to_string(),
            |attrs| {
                let k = get_norm_label(attrs);
                if k.is_empty() { nid.to_string() } else { k }
            },
        )
    };
    let top_score = scored[0].0;
    let mut seeds: Vec<String> = Vec::new();
    let mut seen_labels: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (score, nid) in scored {
        if seeds.len() >= max_k {
            break;
        }
        if !seeds.is_empty() && *score < top_score * gap_ratio {
            break;
        }
        if !seen_labels.insert(seed_label_key(nid)) {
            continue;
        }
        seeds.push(nid.clone());
    }
    // Distinct, sorted query tokens (BTreeSet mirrors Python `sorted({...})`).
    let norm_terms: std::collections::BTreeSet<String> =
        terms.iter().flat_map(|t| search_tokens(t)).collect();
    for term in &norm_terms {
        let term_scored = score_nodes(graph, &[term.as_str()], idf_cache);
        let Some((best_score, first_nid)) = term_scored.first() else {
            continue;
        };
        let best_score = *best_score;
        let tied: Vec<&str> = term_scored
            .iter()
            .filter(|(s, _)| s.total_cmp(&best_score) == std::cmp::Ordering::Equal)
            .map(|(_, n)| n.as_str())
            .collect();
        // On a tie, the highest-degree node wins; keep the FIRST such node
        // (matches Python `max(tied, key=G.degree)`, which returns the first
        // max). Degree here is endpoint incidence — a self-loop counts twice,
        // matching NetworkX `G.degree`, not the once-counting `node_degree`.
        let incidence = |n: &str| -> usize {
            graph
                .edges()
                .map(|e| usize::from(e.source == n) + usize::from(e.target == n))
                .sum()
        };
        let best_nid = if tied.len() > 1 {
            let mut best = tied[0];
            let mut best_deg = incidence(best);
            for &n in &tied[1..] {
                let d = incidence(n);
                if d > best_deg {
                    best = n;
                    best_deg = d;
                }
            }
            best.to_string()
        } else {
            first_nid.clone()
        };
        // Honor the same per-label cap so the per-term guarantee can't
        // reintroduce a second copy of an already-seeded generic label (#1766).
        if !seeds.contains(&best_nid) && seen_labels.insert(seed_label_key(&best_nid)) {
            seeds.push(best_nid);
        }
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

/// Resolve a single token alias to its canonical context name (e.g. `"param"` →
/// `"parameter_type"`, `"decorator"` → `"attribute"`). Returns `None` when the
/// input is already canonical or unknown — callers fall back to the original.
///
/// Mirrors Python `_CONTEXT_FILTER_ALIASES`.
fn context_filter_alias(key: &str) -> Option<&'static str> {
    match key {
        "param" | "params" | "parameter" | "parameters" | "argument" | "arguments" | "arg"
        | "args" => Some("parameter_type"),
        "return" | "returns" | "returned" => Some("return_type"),
        "generic" | "generics" | "template" | "templates" => Some("generic_arg"),
        "annotation" | "annotations" | "decorator" | "decorators" => Some("attribute"),
        "calls" | "called" | "invoke" | "invocation" => Some("call"),
        "fields" | "property" | "properties" | "member" | "members" => Some("field"),
        "imports" | "imported" | "module" | "modules" => Some("import"),
        "exports" | "exported" => Some("export"),
        _ => None,
    }
}

/// Normalise an explicit filter list (deduplicate, strip whitespace, resolve
/// shorthand aliases to their canonical edge-context names).
///
/// Mirrors Python `_normalize_context_filters`.
#[must_use]
pub fn normalize_context_filters(filters: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for value in filters {
        let key = strip_diacritics(value.trim()).to_lowercase();
        if key.is_empty() {
            continue;
        }
        let canonical = context_filter_alias(&key).map_or(key, str::to_owned);
        if seen.insert(canonical.clone()) {
            normalized.push(canonical);
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

/// Resolve a node's displayed community, mirroring Python
/// `str(d.get('community_name') or d.get('community', ''))`.
///
/// Prefers a non-empty `community_name` (the human-readable label, e.g.
/// `"Auth Layer"`) and otherwise falls back to the numeric `community` id.
/// Returns `None` when neither attribute is present, which renders as `""`.
#[must_use]
pub fn community_label(attrs: &IndexMap<String, Value>) -> Option<String> {
    attrs
        .get("community_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            attrs.get("community").and_then(|v| {
                v.as_i64()
                    .map(|n| n.to_string())
                    .or_else(|| v.as_str().map(str::to_owned))
                    .or_else(|| (!v.is_null()).then(|| v.to_string()))
            })
        })
}

/// Work-memory `learning=<status>[:stale]` suffix for a node's query-text NODE
/// line, or `""` when the overlay has no entry / no status (#1441).
fn node_learning_suffix(overlay: Option<&serde_json::Map<String, Value>>, nid: &str) -> String {
    overlay
        .and_then(|o| o.get(nid))
        .and_then(Value::as_object)
        .map_or_else(String::new, |e| {
            let status = graphify_security::sanitize_label(
                e.get("status").and_then(Value::as_str).or(Some("")),
            );
            if status.is_empty() {
                return String::new();
            }
            let stale = if e.get("stale").and_then(Value::as_bool) == Some(true) {
                ":stale"
            } else {
                ""
            };
            format!(" learning={status}{stale}")
        })
}

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

    // Work-memory overlay (#1441): annotate a node with its learned status so the
    // agent sees which sources past sessions found preferred/tentative/contested.
    let overlay = graph
        .graph_attrs
        .get("_learning_overlay")
        .and_then(Value::as_object);
    let mut lines: Vec<String> = Vec::new();
    for nid in &ordered {
        let empty = IndexMap::new();
        let d = graph.node_data(nid).unwrap_or(&empty);
        let learning_suffix = node_learning_suffix(overlay, nid);
        let line = format!(
            "NODE {} [src={} loc={} community={}{learning_suffix}]",
            sanitize_label(d.get("label").and_then(Value::as_str).or(Some(nid))),
            sanitize_label(d.get("source_file").and_then(Value::as_str).or(Some(""))),
            sanitize_label(
                d.get("source_location")
                    .and_then(Value::as_str)
                    .or(Some(""))
            ),
            sanitize_label(community_label(d).as_deref()),
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

/// Return node IDs whose source-file path, label, or ID matches the search term
/// (diacritic-insensitive).
///
/// Ordered: exact source-file path, then exact (label/ID), prefix, substring.
/// When a source-file path matches several nodes (a file node plus the symbols
/// inside it), the L1 file node whose basename equals the query basename is
/// floated to the front so a path query lands on the file, not a symbol (#1503).
///
/// Both the query and the node label/ID are run through [`search_tokens`] so
/// punctuated names (`foo.bar`, `foo()`, `pkg::Type`) match a tokenised query.
/// graphify-py `_find_node` matches this on the *label* side since #1338 (it
/// builds `label_tokens = " ".join(_search_tokens(label))`). The Rust port also
/// tokenises the node ID for exact/prefix/substring matching — a benign superset
/// that gives broader id recall than graphify-py.
#[must_use]
pub fn find_node(graph: &Graph, label: &str) -> Vec<String> {
    let term = search_tokens(label).join(" ");
    if term.is_empty() {
        return Vec::new();
    }
    // Slash-normalize the query once (Windows `\` → `/`) so the basename (for
    // the L1 file-node preference) and the full-path compare share one
    // separator convention; otherwise `src\foo.rs` resolves the file but its
    // basename keeps the backslash and misses the L1 preference (#1503).
    let query_norm = strip_diacritics(label).to_lowercase().replace('\\', "/");
    let query_basename = Path::new(&query_norm)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&query_norm)
        .to_string();
    // Slash-normalized full path of the query, for exact source-path matching.
    // Trailing separators are trimmed so a path query keeps matching the file
    // (parity with the old tokenized compare, which dropped them) (#1503).
    let query_path = query_norm.trim_end_matches('/').to_string();
    // Punctuation-PRESERVING normalized query (#1704): `term` tokenizes on \w+
    // ("blockStream.ts" -> "blockstream ts") but a node's stored `norm_label`
    // keeps punctuation ("blockstream.ts"). Matching `norm_query` against the raw
    // `norm_label`/`bare_label` symmetrically resolves an exactly-typed punctuated
    // label even when `label` and `norm_label` diverge. NOT slash-normalized —
    // that is the separate #1503 path handled by `query_norm` above.
    let norm_query = strip_diacritics(label).to_lowercase().trim().to_string();
    let mut source_exact: Vec<String> = Vec::new();
    let mut preferred: Vec<String> = Vec::new();
    let mut exact: Vec<String> = Vec::new();
    let mut prefix: Vec<String> = Vec::new();
    let mut substring: Vec<String> = Vec::new();

    for (nid, attrs) in graph.nodes() {
        // Fetch the stored norm_label once; derive the tokenized form from it and
        // reuse the raw form for the punctuation-preserving norm_query compare.
        let norm_label_raw = get_norm_label(attrs);
        let bare_label_raw = norm_label_raw.trim_end_matches(['(', ')']);
        // Token-join both sides for the #1503 tokenized match (`search_tokens`
        // strips trailing `()`); `search_tokens` lowercases, so pass `nid` directly.
        let node_term = search_tokens(&norm_label_raw).join(" ");
        let nid_term = search_tokens(nid).join(" ");
        // Match the source-file path on its slash-normalized full form, NOT
        // tokenized. graphify-py compares tokenized source paths (serve.py
        // `source_tokens`), which collapses distinct paths to the same tokens
        // (`src/foo/bar.py` and `src/foo_bar.py` both → "src foo bar py"), so a
        // path query could land on the wrong file. The full-path compare avoids
        // that; tokenized matching stays for label/id below (divergence, #1503).
        let source_path = strip_diacritics(
            attrs
                .get("source_file")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )
        .to_lowercase()
        .replace('\\', "/");
        if !source_path.is_empty() && query_path == source_path {
            source_exact.push(nid.clone());
            if attrs.get("source_location").and_then(Value::as_str) == Some("L1")
                && norm_label_raw == query_basename
            {
                preferred.push(nid.clone());
            }
        } else if term == node_term
            || term == nid_term
            || norm_query == norm_label_raw
            || norm_query == bare_label_raw
        {
            exact.push(nid.clone());
        } else if node_term.starts_with(&term)
            || nid_term.starts_with(&term)
            || norm_label_raw.starts_with(&norm_query)
            || bare_label_raw.starts_with(&norm_query)
        {
            prefix.push(nid.clone());
        } else if node_term.contains(&term)
            || nid_term.contains(&term)
            || norm_label_raw.contains(&norm_query)
        {
            substring.push(nid.clone());
        }
    }

    if let [only] = preferred.as_slice() {
        let mut reordered = vec![only.clone()];
        reordered.extend(
            source_exact
                .iter()
                .filter(|n| n.as_str() != only.as_str())
                .cloned(),
        );
        source_exact = reordered;
    }

    source_exact.extend(exact);
    source_exact.extend(prefix);
    source_exact.extend(substring);
    source_exact
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

/// Return `true` when `c` is a CJK Unified Ideograph (U+4E00..=U+9FFF).
fn is_chinese_char(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c)
}

/// Return `true` when `text` contains at least one CJK Unified Ideograph.
/// Mirrors Python `_has_chinese` (#1026) — the helper is scoped to Chinese
/// on purpose: Hiragana/Katakana/Hangul segmentation accuracy with bigrams
/// is poor and `jieba`-style dictionaries don't ship with that script
/// support, so non-Chinese non-ASCII text falls through as a single term.
#[must_use]
fn has_chinese(text: &str) -> bool {
    text.chars().any(is_chinese_char)
}

/// Bigram-based Chinese segmentation fallback.
///
/// Walks `text` collecting bigrams only **within** contiguous runs of
/// Chinese characters, then appends the original unsegmented term so
/// exact-substring searches still hit.
///
/// Divergence from `graphify-py` `_segment_chinese` (intentional): the
/// Python bigram fallback walks raw character pairs without checking
/// script boundaries, so an input like `"a前b"` produces noisy
/// cross-script bigrams `["a前", "前b"]`. That noise is invisible in
/// Python because jieba (when installed) tokenises mixed-script input
/// directly, but the Rust port has only the bigram path. Run-scoped
/// bigrams keep the search terms relevant — single-character Chinese
/// runs surrounded by other scripts emit no bigrams (no useful pair to
/// emit) and the original term is preserved either way.
#[must_use]
fn segment_chinese_bigram(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut segments: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if is_chinese_char(chars[i]) {
            let mut j = i;
            while j < chars.len() && is_chinese_char(chars[j]) {
                j += 1;
            }
            // Bigrams within this Chinese run only.
            if j - i >= 2 {
                for k in i..j - 1 {
                    segments.push(chars[k..=k + 1].iter().collect());
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    // Append the unsegmented term so an exact match against an indexed
    // label still resolves. Mirrors `_segment_chinese`'s `text not in
    // segments` tail-append (and gives us a search term even when the
    // run-scoped walk produced none — e.g. for `"a前b"`).
    if chars.len() > 1 && !segments.iter().any(|s| s == text) {
        segments.push(text.to_string());
    }
    if segments.is_empty() {
        segments.push(text.to_string());
    }
    segments
}

/// Return `true` if `term` should survive the short-English filter — Chinese
/// (and any other non-ASCII) terms pass; ASCII-lowercase terms must exceed
/// two characters. Mirrors Python `_is_searchable`.
#[must_use]
fn is_searchable(term: &str) -> bool {
    if term.chars().all(|c| c.is_ascii_lowercase()) {
        return term.chars().count() > 2;
    }
    true
}

/// English question/filler words dropped from query terms so content words
/// drive BFS seeding: "how does the frontier cache work" seeds on
/// `frontier`/`cache`, not `how`/`the`/`work` (which prefix-match prose labels
/// at the 100x tier). Applied to query terms only — node text is never
/// filtered, so a symbol literally named `work` stays findable via
/// explain/path. `work`/`works`/`working` are included as the most common
/// question phrasing ("how does X work").
static QUERY_STOPWORDS: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "how", "what", "why", "when", "where", "which", "who", "whom", "whose", "does", "did",
            "is", "are", "was", "were", "be", "been", "being", "can", "could", "should", "would",
            "will", "shall", "may", "might", "must", "has", "have", "had", "the", "and", "but",
            "not", "for", "from", "with", "without", "into", "onto", "off", "that", "this",
            "these", "those", "there", "here", "its", "their", "them", "they", "about", "any",
            "all", "some", "work", "works", "working",
        ]
        .into_iter()
        .collect()
    });

/// Split a query string into searchable terms.
///
/// Terms are lowercased; short tokens (≤ 2 chars) are dropped only when
/// they are entirely English (ASCII `a-z`). Chinese sub-strings are
/// bigram-segmented so substring matches resolve against indexed labels.
/// Mirrors Python `_query_terms` in `serve.py` (#964, #1026).
#[must_use]
pub fn query_terms(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in question.split_whitespace() {
        if has_chinese(raw) {
            let lower = raw.to_lowercase();
            for seg in segment_chinese_bigram(lower.trim()) {
                let trimmed = seg.trim();
                if !trimmed.is_empty() && is_searchable(trimmed) {
                    out.push(trimmed.to_string());
                }
            }
        } else {
            // Strip punctuation without touching Unicode characters (avoid NFKD
            // mangling non-Latin scripts). Mirrors graphify-py `_query_terms`.
            let lower = raw.to_lowercase();
            for tok in word_tokens(&lower) {
                if is_searchable(tok) {
                    out.push(tok.to_string());
                }
            }
        }
    }
    // Drop question/filler words so content words drive seeding, falling back to
    // the unfiltered terms when the query is all stopwords ("how does it work").
    let content: Vec<String> = out
        .iter()
        .filter(|t| !QUERY_STOPWORDS.contains(t.as_str()))
        .cloned()
        .collect();
    if content.is_empty() { out } else { content }
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
    let start_nodes = pick_seeds_diverse(&scored, 3, 0.2, graph, &term_refs, idf_cache);
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
