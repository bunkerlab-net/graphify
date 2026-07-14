//! MCP tool handler implementations.
//!
//! Each function mirrors a `_tool_*` inner function from `graphify-py/graphify/serve.py`.
//! Handlers receive a parsed `serde_json::Map` of tool arguments and return a `String`.

use std::collections::HashMap;
use std::path::Path;

use graphify_build::Graph;
use graphify_prs::gh::GhClient;
use graphify_prs::git::GitClient;
use graphify_prs::graph::{FileIndex, build_file_index, compute_pr_impact};
use graphify_security::sanitize_label;
use indexmap::IndexMap;
use serde_json::Value;

use crate::ReloadState;
use crate::ServeError;
use crate::graph::{
    community_label, find_node, node_degree, predecessors, query_graph_text, score_nodes,
    shortest_path, successors,
};

/// Execute the `query_graph` tool.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // depth/budget casts are bounded by tool config.
pub fn tool_query_graph<S: std::hash::BuildHasher>(
    graph: &Graph,
    arguments: &serde_json::Map<String, Value>,
    idf_cache: &mut HashMap<String, f64, S>,
) -> String {
    let question = match arguments.get("question").and_then(Value::as_str) {
        Some(q) => q.to_string(),
        None => return "Error: 'question' argument is required.".to_string(),
    };
    let mode = arguments
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("bfs");
    let depth = arguments
        .get("depth")
        .and_then(Value::as_u64)
        .map_or(3, |d| d.min(6) as usize);
    let budget = arguments
        .get("token_budget")
        .and_then(Value::as_u64)
        .map_or(2000, |b| b as usize);
    let context_filter: Option<Vec<String>> = arguments
        .get("context_filter")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        });

    query_graph_text(
        graph,
        &question,
        mode,
        depth,
        budget,
        context_filter.as_deref(),
        idf_cache,
    )
}

/// Execute the `get_node` tool.
#[must_use]
pub fn tool_get_node(graph: &Graph, arguments: &serde_json::Map<String, Value>) -> String {
    let label = match arguments.get("label").and_then(Value::as_str) {
        Some(l) => l.to_lowercase(),
        None => return "Error: 'label' argument is required.".to_string(),
    };
    let matches: Vec<(&String, &IndexMap<String, Value>)> = graph
        .nodes()
        .filter(|(nid, d)| {
            d.get("label")
                .and_then(Value::as_str)
                .is_some_and(|l| l.to_lowercase().contains(&label))
                || nid.to_lowercase() == label
        })
        .collect();
    if matches.is_empty() {
        return format!("No node matching '{label}' found.");
    }
    let (nid, d) = matches[0];
    [
        format!(
            "Node: {}",
            sanitize_label(d.get("label").and_then(Value::as_str).or(Some(nid)))
        ),
        format!("  ID: {}", sanitize_label(Some(nid))),
        format!(
            "  Source: {} {}",
            sanitize_label(d.get("source_file").and_then(Value::as_str).or(Some(""))),
            sanitize_label(
                d.get("source_location")
                    .and_then(Value::as_str)
                    .or(Some(""))
            )
        ),
        format!(
            "  Type: {}",
            sanitize_label(d.get("file_type").and_then(Value::as_str).or(Some("")))
        ),
        format!(
            "  Community: {}",
            sanitize_label(community_label(d).as_deref())
        ),
        format!("  Degree: {}", node_degree(graph, nid)),
    ]
    .join("\n")
}

/// Execute the `get_neighbors` tool.
#[must_use]
pub fn tool_get_neighbors(graph: &Graph, arguments: &serde_json::Map<String, Value>) -> String {
    let label = match arguments.get("label").and_then(Value::as_str) {
        Some(l) => l.to_lowercase(),
        None => return "Error: 'label' argument is required.".to_string(),
    };
    let rel_filter = arguments
        .get("relation_filter")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let matches = find_node(graph, &label);
    if matches.is_empty() {
        return format!("No node matching '{label}' found.");
    }
    let nid = &matches[0];
    let empty = IndexMap::new();
    let node_label = graph
        .node_data(nid)
        .unwrap_or(&empty)
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or(nid);
    let mut lines = vec![format!(
        "Neighbors of {}:",
        sanitize_label(Some(node_label))
    )];

    for nb in successors(graph, nid) {
        let d = graph.edge_data(nid, &nb).cloned().unwrap_or_default();
        let rel = d.get("relation").and_then(Value::as_str).unwrap_or("");
        if !rel_filter.is_empty() && !rel.to_lowercase().contains(&rel_filter) {
            continue;
        }
        let empty_node = IndexMap::new();
        let nb_label = graph
            .node_data(&nb)
            .unwrap_or(&empty_node)
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(&nb);
        let conf = d.get("confidence").and_then(Value::as_str).unwrap_or("");
        lines.push(format!(
            "  --> {} [{}] [{}]",
            sanitize_label(Some(nb_label)),
            sanitize_label(Some(rel)),
            sanitize_label(Some(conf)),
        ));
    }

    if graph.kind.is_directed() {
        for nb in predecessors(graph, nid) {
            let d = graph.edge_data(&nb, nid).cloned().unwrap_or_default();
            let rel = d.get("relation").and_then(Value::as_str).unwrap_or("");
            if !rel_filter.is_empty() && !rel.to_lowercase().contains(&rel_filter) {
                continue;
            }
            let empty_node = IndexMap::new();
            let nb_label = graph
                .node_data(&nb)
                .unwrap_or(&empty_node)
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(&nb);
            let conf = d.get("confidence").and_then(Value::as_str).unwrap_or("");
            lines.push(format!(
                "  <-- {} [{}] [{}]",
                sanitize_label(Some(nb_label)),
                sanitize_label(Some(rel)),
                sanitize_label(Some(conf)),
            ));
        }
    }
    lines.join("\n")
}

/// Header line for `get_community`: `"Community N — Name"` when the community
/// has a real label, else the bare `"Community N"`.
///
/// Skips the name when it is just the `"Community N"` placeholder (written for
/// unnamed communities) so the header never reads `"Community 12 — Community
/// 12"`. The name is sanitised like every other LLM-derived field. Ports
/// Python `_community_header` (#1448).
#[must_use]
pub fn community_header(cid: i64, community_name: Option<&str>) -> String {
    let base = format!("Community {cid}");
    if let Some(name) = community_name {
        let clean = sanitize_label(Some(name));
        if !clean.is_empty() && clean != base {
            return format!("{base} — {clean}");
        }
    }
    base
}

/// Execute the `get_community` tool.
#[must_use]
pub fn tool_get_community(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    arguments: &serde_json::Map<String, Value>,
) -> String {
    let Some(cid) = arguments.get("community_id").and_then(Value::as_i64) else {
        return "Error: 'community_id' argument is required.".to_string();
    };
    let nodes = match communities.get(&cid) {
        Some(ns) if !ns.is_empty() => ns,
        _ => return format!("Community {cid} not found."),
    };
    let community_name = graph
        .node_data(&nodes[0])
        .and_then(|d| d.get("community_name"))
        .and_then(Value::as_str);
    let header = community_header(cid, community_name);
    let mut lines = vec![format!("{header} ({} nodes):", nodes.len())];
    for n in nodes {
        let empty = IndexMap::new();
        let d = graph.node_data(n).unwrap_or(&empty);
        lines.push(format!(
            "  {} [{}]",
            sanitize_label(d.get("label").and_then(Value::as_str).or(Some(n))),
            sanitize_label(d.get("source_file").and_then(Value::as_str).or(Some(""))),
        ));
    }
    lines.join("\n")
}

/// Execute the `god_nodes` tool.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // top_n is a small UI count; truncation is safe.
pub fn tool_god_nodes(graph: &Graph, arguments: &serde_json::Map<String, Value>) -> String {
    let top_n = arguments
        .get("top_n")
        .and_then(Value::as_u64)
        .map_or(10, |n| n as usize);
    let nodes = graphify_analyze::god_nodes(graph, top_n);
    let mut lines = vec!["God nodes (most connected):".to_string()];
    for (i, n) in nodes.iter().enumerate() {
        let label = n.get("label").and_then(Value::as_str).unwrap_or("");
        let degree = n.get("degree").and_then(Value::as_u64).unwrap_or(0);
        lines.push(format!("  {}. {label} - {degree} edges", i + 1));
    }
    lines.join("\n")
}

/// Execute the `graph_stats` tool.
#[must_use]
// cast_precision_loss/cast_possible_truncation/cast_sign_loss: percentage rounded to
// nearest integer before casting to u64; all values are in [0, 100].
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn tool_graph_stats(graph: &Graph, communities: &IndexMap<i64, Vec<String>>) -> String {
    let confs: Vec<Option<&str>> = graph
        .edges()
        .map(|e| e.attrs.get("confidence").and_then(Value::as_str))
        .collect();
    let total = confs.len().max(1);
    let count = |label: &str| confs.iter().filter(|c| **c == Some(label)).count();
    let pct = |label: &str| (count(label) as f64 / total as f64 * 100.0).round() as u64;
    format!(
        "Nodes: {}\nEdges: {}\nCommunities: {}\nEXTRACTED: {}%\nINFERRED: {}%\nAMBIGUOUS: {}%\n",
        graph.node_count(),
        graph.edge_count(),
        communities.len(),
        pct("EXTRACTED"),
        pct("INFERRED"),
        pct("AMBIGUOUS"),
    )
}

/// Execute the `shortest_path` tool.
#[must_use]
// too_many_lines: port of complex Python function; splitting would harm readability.
// implicit_hasher: IDF cache is owned by callers; concrete HashMap intentional.
// cast_possible_truncation: max_hops is a small UI count.
#[allow(
    clippy::too_many_lines,
    clippy::implicit_hasher,
    clippy::cast_possible_truncation
)]
pub fn tool_shortest_path(
    graph: &Graph,
    arguments: &serde_json::Map<String, Value>,
    idf_cache: &mut HashMap<String, f64>,
) -> String {
    let source_q = match arguments.get("source").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return "Error: 'source' argument is required.".to_string(),
    };
    let target_q = match arguments.get("target").and_then(Value::as_str) {
        Some(t) => t.to_string(),
        None => return "Error: 'target' argument is required.".to_string(),
    };

    let src_terms: Vec<String> = source_q.split_whitespace().map(str::to_lowercase).collect();
    let tgt_terms: Vec<String> = target_q.split_whitespace().map(str::to_lowercase).collect();
    let src_refs: Vec<&str> = src_terms.iter().map(String::as_str).collect();
    let tgt_refs: Vec<&str> = tgt_terms.iter().map(String::as_str).collect();

    let src_scored = score_nodes(graph, &src_refs, idf_cache);
    let tgt_scored = score_nodes(graph, &tgt_refs, idf_cache);

    if src_scored.is_empty() {
        return format!("No node matching source '{source_q}' found.");
    }
    if tgt_scored.is_empty() {
        return format!("No node matching target '{target_q}' found.");
    }

    let src_nid = &src_scored[0].1;
    let tgt_nid = &tgt_scored[0].1;

    if src_nid == tgt_nid {
        return format!(
            "'{source_q}' and '{target_q}' both resolved to the same node '{src_nid}'. \
Use a more specific label or the exact node ID."
        );
    }

    let mut warnings: Vec<String> = Vec::new();
    for (name, scored) in [("source", &src_scored), ("target", &tgt_scored)] {
        if scored.len() >= 2 {
            let top = scored[0].0;
            let runner = scored[1].0;
            if top > 0.0 && (top - runner) / top < 0.10 {
                warnings.push(format!(
                    "warning: {name} match was ambiguous (top score {top}, runner-up {runner})"
                ));
            }
        }
    }

    let max_hops = arguments
        .get("max_hops")
        .and_then(Value::as_u64)
        .map_or(8, |h| h as usize);

    let Some(path_nodes) = shortest_path(graph, src_nid, tgt_nid) else {
        let empty = IndexMap::new();
        let src_label = graph
            .node_data(src_nid)
            .unwrap_or(&empty)
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(src_nid);
        let tgt_label = graph
            .node_data(tgt_nid)
            .unwrap_or(&empty)
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(tgt_nid);
        return format!("No path found between '{src_label}' and '{tgt_label}'.");
    };

    let hops = path_nodes.len().saturating_sub(1);
    if hops > max_hops {
        return format!("Path exceeds max_hops={max_hops} ({hops} hops found).");
    }

    let mut segments: Vec<String> = Vec::new();
    for i in 0..path_nodes.len().saturating_sub(1) {
        let u = &path_nodes[i];
        let v = &path_nodes[i + 1];
        let (edata, forward) = match graph.edge_data(u, v).cloned() {
            Some(e) => (e, true),
            None => (graph.edge_data(v, u).cloned().unwrap_or_default(), false),
        };
        let rel = edata.get("relation").and_then(Value::as_str).unwrap_or("");
        let conf = edata
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("");
        let conf_str = if conf.is_empty() {
            String::new()
        } else {
            format!(" [{conf}]")
        };
        let empty = IndexMap::new();
        if i == 0 {
            let u_label = graph
                .node_data(u)
                .unwrap_or(&empty)
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(u);
            segments.push(u_label.to_string());
        }
        let v_label = graph
            .node_data(v)
            .unwrap_or(&empty)
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(v);
        if forward {
            segments.push(format!("--{rel}{conf_str}--> {v_label}"));
        } else {
            segments.push(format!("<--{rel}{conf_str}-- {v_label}"));
        }
    }

    let prefix = if warnings.is_empty() {
        String::new()
    } else {
        warnings.join("\n") + "\n"
    };
    format!(
        "{prefix}Shortest path ({hops} hops):\n  {}",
        segments.join(" ")
    )
}

// ── PR tools ──────────────────────────────────────────────────────────────────

/// List open PRs with CI status and graph impact.
///
/// Mirrors Python `_tool_list_prs`. Calls [`graphify_prs::fetch_prs`] then
/// formats the result with [`graphify_prs::format_prs_text`].
///
/// # Errors
///
/// Returns `ServeError` when `gh` is unavailable or returns invalid data.
pub fn tool_list_prs(
    args: &Value,
    gh: &dyn GhClient,
    git: &dyn GitClient,
) -> Result<Value, ServeError> {
    tool_list_prs_with_clients(args, gh, git)
}

/// Inner implementation — separated for testability with injected clients.
///
/// # Errors
///
/// Returns `ServeError` on `gh` failures.
pub fn tool_list_prs_with_clients(
    args: &Value,
    gh: &dyn GhClient,
    git: &dyn GitClient,
) -> Result<Value, ServeError> {
    let repo = args.get("repo").and_then(Value::as_str);
    let base = args.get("base").and_then(Value::as_str);
    // cast_possible_truncation: limit is a small UI count; 32-bit truncation is safe.
    #[allow(clippy::cast_possible_truncation)]
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(50, |n| n as usize);

    let prs = graphify_prs::fetch_prs(gh, git, repo, base, limit)
        .map_err(|e| ServeError::Io(e.to_string()))?;

    let items: Vec<Value> = prs
        .iter()
        .map(|pr| {
            serde_json::json!({
                "number": pr.number,
                "title": pr.title,
                "branch": pr.branch,
                "base_branch": pr.base_branch,
                "author": pr.author,
                "status": pr.status(),
                "ci_status": pr.ci_status,
                "review_decision": pr.review_decision,
                "days_old": pr.days_old(),
                "draft": pr.is_draft,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "count": items.len(),
        "prs": items,
    }))
}

/// Get detailed graph impact for a single PR.
///
/// Mirrors Python `_tool_get_pr_impact`. Fetches the PR's changed files via
/// the `gh` client, then intersects them with the in-memory graph index to
/// identify affected nodes and communities.
///
/// # Errors
///
/// Returns `ServeError` when `gh` is unavailable or returns invalid data.
pub fn tool_get_pr_impact(
    graph: &Graph,
    args: &Value,
    gh: &dyn GhClient,
) -> Result<Value, ServeError> {
    tool_get_pr_impact_with_clients(graph, args, gh)
}

/// Inner implementation — separated for testability with injected clients.
///
/// # Errors
///
/// Returns `ServeError` on `gh` failures.
pub fn tool_get_pr_impact_with_clients(
    graph: &Graph,
    args: &Value,
    gh: &dyn GhClient,
) -> Result<Value, ServeError> {
    let pr_number = args
        .get("pr_number")
        .and_then(Value::as_u64)
        .ok_or_else(|| ServeError::Io("'pr_number' argument is required".to_string()))?;
    let repo = args.get("repo").and_then(Value::as_str);

    let files = gh.pr_files(pr_number, repo);

    // Build the file index from graph node data for impact computation.
    let nodes: Vec<Value> = graph
        .nodes()
        .map(|(_, data)| {
            let mut obj = serde_json::Map::new();
            for (k, v) in data {
                obj.insert(k.clone(), v.clone());
            }
            Value::Object(obj)
        })
        .collect();
    let index: FileIndex = build_file_index(&nodes);
    let (communities, nodes_affected) = compute_pr_impact(&files, &index);

    Ok(serde_json::json!({
        "pr_number": pr_number,
        "files_changed": files,
        "affected_nodes": nodes_affected,
        "communities_touched": communities,
    }))
}

/// Return actionable open PRs sorted by review priority with graph impact data.
///
/// Mirrors Python `_tool_triage_prs`. Uses [`graphify_prs::triage::NoOpTriageBackend`]
/// as the LLM backend (no actual AI ranking is performed; the raw structured
/// data is returned for the caller to reason about).
///
/// # Errors
///
/// Returns `ServeError` when `gh` is unavailable or returns invalid data.
pub fn tool_triage_prs(
    args: &Value,
    gh: &dyn GhClient,
    git: &dyn GitClient,
) -> Result<Value, ServeError> {
    tool_triage_prs_with_clients(args, gh, git)
}

/// Inner implementation — separated for testability with injected clients.
///
/// # Errors
///
/// Returns `ServeError` on `gh` failures.
pub fn tool_triage_prs_with_clients(
    args: &Value,
    gh: &dyn GhClient,
    git: &dyn GitClient,
) -> Result<Value, ServeError> {
    let repo = args.get("repo").and_then(Value::as_str);
    let base_arg = args.get("base").and_then(Value::as_str);
    // cast_possible_truncation: limit is a small UI count; 32-bit truncation is safe.
    #[allow(clippy::cast_possible_truncation)]
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(usize::MAX, |n| n as usize);

    let prs = graphify_prs::fetch_prs(gh, git, repo, base_arg, 50)
        .map_err(|e| ServeError::Io(e.to_string()))?;

    let base = base_arg.map_or_else(
        || graphify_prs::detect_default_branch(gh, git, repo),
        str::to_string,
    );

    // Keep only actionable PRs (right base, not stale, not wrong-base).
    let actionable: Vec<&graphify_prs::PrInfo> = prs
        .iter()
        .filter(|p| {
            let s = p.status();
            p.base_branch == base && s != "WRONG-BASE" && s != "STALE"
        })
        .take(limit)
        .collect();

    // Sort by status priority order.
    let mut sorted: Vec<&graphify_prs::PrInfo> = actionable;
    sorted.sort_by_key(|p| {
        graphify_prs::model::STATUS_ORDER
            .iter()
            .position(|&s| s == p.status().as_str())
            .unwrap_or(99)
    });

    let items: Vec<Value> = sorted
        .iter()
        .map(|pr| {
            serde_json::json!({
                "number": pr.number,
                "title": pr.title,
                "branch": pr.branch,
                "status": pr.status(),
                "ci_status": pr.ci_status,
                "review_decision": pr.review_decision,
                "days_old": pr.days_old(),
                "author": pr.author,
                "nodes_affected": pr.nodes_affected,
                "communities_touched": pr.communities_touched,
                "blast_radius": pr.blast_radius(),
            })
        })
        .collect();

    Ok(Value::Array(items))
}

/// Render the `graphify://audit` resource.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // Percentage rounding; all values are in [0, 100].
pub fn resource_audit(graph: &Graph) -> String {
    let confs: Vec<Option<&str>> = graph
        .edges()
        .map(|e| e.attrs.get("confidence").and_then(Value::as_str))
        .collect();
    let total = confs.len().max(1);
    let count = |label: &str| confs.iter().filter(|c| **c == Some(label)).count();
    let pct = |label: &str| (count(label) as f64 / total as f64 * 100.0).round() as u64;
    format!(
        "Total edges: {total}\nEXTRACTED: {} ({}%)\nINFERRED: {} ({}%)\nAMBIGUOUS: {} ({}%)\n",
        count("EXTRACTED"),
        pct("EXTRACTED"),
        count("INFERRED"),
        pct("INFERRED"),
        count("AMBIGUOUS"),
        pct("AMBIGUOUS"),
    )
}

/// Render the `graphify://surprises` resource.
#[must_use]
pub fn resource_surprises(graph: &Graph, communities: &IndexMap<i64, Vec<String>>) -> String {
    let surprises = graphify_analyze::surprising_connections(graph, communities, 10);
    if surprises.is_empty() {
        return "No surprising connections found.".to_string();
    }
    let mut lines = vec!["Surprising cross-community connections:".to_string()];
    for s in &surprises {
        let src = s.get("source").and_then(Value::as_str).unwrap_or("");
        let tgt = s.get("target").and_then(Value::as_str).unwrap_or("");
        let rel = s.get("relation").and_then(Value::as_str).unwrap_or("");
        lines.push(format!("  {src} <-> {tgt} [{rel}]"));
    }
    lines.join("\n")
}

/// Render the `graphify://questions` resource.
#[must_use]
pub fn resource_questions(
    graph: &Graph,
    communities: &IndexMap<i64, Vec<String>>,
    community_labels: &IndexMap<i64, String>,
) -> String {
    let questions = graphify_analyze::suggest_questions(graph, communities, community_labels, 10);
    if questions.is_empty() {
        return "No suggested questions available.".to_string();
    }
    let mut lines = vec!["Suggested questions:".to_string()];
    for q in &questions {
        let text = q.get("question").and_then(Value::as_str).unwrap_or("");
        lines.push(format!("  - {text}"));
    }
    lines.join("\n")
}

/// Load community labels from `.graphify_labels.json` next to `graph_path`.
#[must_use]
pub fn load_community_labels(
    graph_path: &str,
    communities: &IndexMap<i64, Vec<String>>,
) -> IndexMap<i64, String> {
    let labels_path = Path::new(graph_path)
        .parent()
        .map(|p| p.join(".graphify_labels.json"));
    if let Some(p) = labels_path
        && p.exists()
        && let Ok(text) = std::fs::read_to_string(&p)
        && let Ok(Value::Object(map)) = serde_json::from_str(&text)
    {
        let mut out = IndexMap::new();
        for (k, v) in map {
            if let (Ok(id), Some(label)) = (k.parse::<i64>(), v.as_str()) {
                out.insert(id, label.to_string());
            }
        }
        return out;
    }
    communities
        .keys()
        .map(|cid| (*cid, format!("Community {cid}")))
        .collect()
}

/// Check if `graph_path` has changed (`mtime_ns` + size), and if so reload.
///
/// Mirrors Python `_maybe_reload`. Returns `true` when the file changed and was
/// successfully reloaded, so callers can invalidate derived caches (e.g. IDF).
#[must_use]
pub fn maybe_reload(
    graph_path: &str,
    graph: &mut Graph,
    communities: &mut IndexMap<i64, Vec<String>>,
    reload_state: &mut ReloadState,
) -> bool {
    use crate::graph::{communities_from_graph, load_graph};

    let Ok(meta) = std::fs::metadata(graph_path) else {
        return false;
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| {
            u64::from(d.subsec_nanos()) + d.as_secs() * 1_000_000_000
        });
    let size = meta.len();
    if (mtime, size) == (reload_state.mtime_ns, reload_state.size) {
        return false;
    }
    // Reload.
    if let Ok(new_g) = load_graph(graph_path) {
        let new_comms = communities_from_graph(&new_g);
        *graph = new_g;
        *communities = new_comms;
        reload_state.mtime_ns = mtime;
        reload_state.size = size;
        return true;
    }
    false
}
