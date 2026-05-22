//! `query`, `path`, and `explain` commands — graph traversal and inspection.

use anyhow::Result;

use crate::cli::{default_graph_path, load_graph};

/// BFS/DFS traversal of `graph.json` to find context relevant to `question`.
///
/// Delegates to `graphify_serve::graph::query_graph_text` and prints the
/// plain-text result. Mirrors Python `__main__.py`'s `query` command.
pub(crate) fn cmd_query(
    question: &str,
    dfs: bool,
    context: &[String],
    budget: usize,
    graph: Option<&std::path::Path>,
) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    eprintln!("loading {} ...", path.display());
    let g = load_graph(&path)?;
    eprintln!(
        "querying {} ({} nodes, mode={}) ...",
        path.display(),
        g.node_count(),
        if dfs { "dfs" } else { "bfs" }
    );
    let mode = if dfs { "dfs" } else { "bfs" };
    let context_filters: Option<&[String]> = if context.is_empty() {
        None
    } else {
        Some(context)
    };
    let mut idf_cache = std::collections::HashMap::new();
    let result = graphify_serve::graph::query_graph_text(
        &g,
        question,
        mode,
        2,
        budget,
        context_filters,
        &mut idf_cache,
    );
    println!("{result}");
    Ok(())
}

/// Print the shortest path between two nodes in `graph.json`.
///
/// Uses `graphify_serve::graph::score_nodes` for label resolution and
/// `shortest_path` (BFS on undirected view) for traversal. Output mirrors
/// Python's `path` command: hop count line + arrow segments tagged with
/// relation + confidence, with an ambiguity guard when both labels resolve
/// to the same node and an ambiguous-match warning to stderr.
pub(crate) fn cmd_path(from: &str, to: &str, graph: Option<&std::path::Path>) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    let g = load_graph(&path)?;

    let mut idf_cache = std::collections::HashMap::new();
    let from_terms: Vec<&str> = from.split_whitespace().collect();
    let to_terms: Vec<&str> = to.split_whitespace().collect();
    let src_scored = graphify_serve::graph::score_nodes(&g, &from_terms, &mut idf_cache);
    let tgt_scored = graphify_serve::graph::score_nodes(&g, &to_terms, &mut idf_cache);

    let Some((_, src_id)) = src_scored.first() else {
        anyhow::bail!("No node matching '{from}' found.");
    };
    let Some((_, tgt_id)) = tgt_scored.first() else {
        anyhow::bail!("No node matching '{to}' found.");
    };

    // Ambiguity guard: both queries collapsed to the same node.
    if src_id == tgt_id {
        anyhow::bail!(
            "'{from}' and '{to}' both resolved to the same node '{src_id}'. \
             Use a more specific label or the exact node ID."
        );
    }

    // Warn on close-runner ambiguity (matches Python's 10% threshold).
    for (name, scored) in [("source", &src_scored), ("target", &tgt_scored)] {
        if scored.len() >= 2 {
            let top = scored[0].0;
            let runner = scored[1].0;
            if top > 0.0 && (top - runner) / top < 0.10 {
                eprintln!(
                    "warning: {name} match was ambiguous (top score {top}, runner-up {runner})"
                );
            }
        }
    }

    let Some(path_nodes) = graphify_serve::graph::shortest_path(&g, src_id, tgt_id) else {
        println!("No path found between '{from}' and '{to}'.");
        return Ok(());
    };

    let hops = path_nodes.len().saturating_sub(1);
    let mut segments: Vec<String> = Vec::new();
    for i in 0..path_nodes.len().saturating_sub(1) {
        let u = &path_nodes[i];
        let v = &path_nodes[i + 1];
        // Determine which way the stored edge points (graph is directed).
        let (edata, forward) = if let Some(d) = g.edge_data(u, v) {
            (Some(d), true)
        } else if let Some(d) = g.edge_data(v, u) {
            (Some(d), false)
        } else {
            (None, true)
        };
        let rel = edata
            .and_then(|d| d.get("relation"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let conf = edata
            .and_then(|d| d.get("confidence"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let conf_str = if conf.is_empty() {
            String::new()
        } else {
            format!(" [{conf}]")
        };
        if i == 0 {
            let u_label = g
                .node_data(u)
                .and_then(|n| n.get("label"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(u);
            segments.push(u_label.to_string());
        }
        let v_label = g
            .node_data(v)
            .and_then(|n| n.get("label"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(v);
        if forward {
            segments.push(format!("--{rel}{conf_str}--> {v_label}"));
        } else {
            segments.push(format!("<--{rel}{conf_str}-- {v_label}"));
        }
    }
    println!("Shortest path ({hops} hops):\n  {}", segments.join(" "));
    Ok(())
}

/// Print a node's metadata + sorted in/out connections from `graph.json`.
///
/// Mirrors Python's `explain` command at `__main__.py:1662`: prints label,
/// ID, source location, `file_type`, community, degree, then up to 20
/// connections sorted by neighbor degree (highest first), tagged with
/// arrows (`-->` for outgoing, `<--` for incoming), relation, confidence.
pub(crate) fn cmd_explain(node: &str, graph: Option<&std::path::Path>) -> Result<()> {
    let path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    let g = load_graph(&path)?;
    let ids = graphify_serve::graph::find_node(&g, node);
    let Some(node_id) = ids.first() else {
        println!("No node matching '{node}' found.");
        return Ok(());
    };
    let attrs = g.node_data(node_id);
    let get_str = |key: &str| -> &str {
        attrs
            .and_then(|a| a.get(key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    };
    let label = get_str("label");
    let label_or_id = if label.is_empty() {
        node_id.as_str()
    } else {
        label
    };
    let source_file = get_str("source_file");
    let source_loc = get_str("source_location");
    let file_type = get_str("file_type");
    let community = attrs
        .and_then(|a| a.get("community"))
        .map(|v| {
            v.as_i64()
                .map(|i| i.to_string())
                .or_else(|| v.as_str().map(str::to_string))
                .unwrap_or_else(|| v.to_string())
        })
        .unwrap_or_default();
    let degree = graphify_serve::graph::node_degree(&g, node_id);

    println!("Node: {label_or_id}");
    println!("  ID:        {node_id}");
    let src_line = format!("  Source:    {source_file} {source_loc}");
    println!("{}", src_line.trim_end());
    println!("  Type:      {file_type}");
    println!("  Community: {community}");
    println!("  Degree:    {degree}");

    // (direction, neighbor_id, edge_data) — but record direction now and look
    // up edge data lazily because we need the *correct* direction's attrs.
    let mut connections: Vec<(&'static str, String)> = Vec::new();
    for nb in graphify_serve::graph::successors(&g, node_id) {
        connections.push(("out", nb));
    }
    for nb in graphify_serve::graph::predecessors(&g, node_id) {
        connections.push(("in", nb));
    }
    if connections.is_empty() {
        return Ok(());
    }

    // Sort by neighbor degree, highest first.
    connections.sort_by(|a, b| {
        graphify_serve::graph::node_degree(&g, &b.1)
            .cmp(&graphify_serve::graph::node_degree(&g, &a.1))
    });

    println!("\nConnections ({}):", connections.len());
    let total = connections.len();
    for (direction, nb) in connections.iter().take(20) {
        let edata = if *direction == "out" {
            g.edge_data(node_id, nb)
        } else {
            g.edge_data(nb, node_id)
        };
        let rel = edata
            .and_then(|d| d.get("relation"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let conf = edata
            .and_then(|d| d.get("confidence"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let arrow = if *direction == "out" { "-->" } else { "<--" };
        let nb_label = g
            .node_data(nb)
            .and_then(|n| n.get("label"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(nb.as_str());
        println!("  {arrow} {nb_label} [{rel}] [{conf}]");
    }
    if total > 20 {
        println!("  ... and {} more", total - 20);
    }
    Ok(())
}
