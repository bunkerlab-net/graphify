//! `tree` command — emit a D3 v7 collapsible-tree HTML for graph.json.

use anyhow::Result;

use crate::cli::{default_graph_path, load_graph};

/// Render a D3 collapsible-tree HTML page from `graph.json`.
///
/// When `max_children` or `label` are non-default, we call `build_tree` directly
/// so we can pass those parameters.  `top_k_edges` is forwarded as a no-op with a
/// warning because the Rust `tree.rs` does not yet expose a top-k parameter on its
/// public API; the flag is accepted so scripts that pass it do not break.
pub(crate) fn cmd_tree(
    graph: Option<&std::path::Path>,
    output: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
    max_children: usize,
    top_k_edges: usize,
    label: Option<&str>,
) -> Result<()> {
    let graph_path = graph.map_or_else(default_graph_path, std::path::Path::to_path_buf);
    eprintln!("loading {} ...", graph_path.display());
    let g = load_graph(&graph_path)?;
    let default_root = std::env::current_dir()?;
    let root_path = root.unwrap_or(default_root.as_path());
    let default_output = graph_path.with_file_name("GRAPH_TREE.html");
    let out = output.unwrap_or(default_output.as_path());
    eprintln!(
        "rendering tree HTML for {} nodes rooted at {} ...",
        g.node_count(),
        root_path.display()
    );
    // `top_k_edges` is not yet exposed on the Rust tree API; warn the user so they
    // are not silently misled, but continue so scripts do not break.
    if top_k_edges != 12 {
        eprintln!(
            "warning: --top-k-edges={top_k_edges} accepted but is currently a no-op \
             (graphify_html::tree does not yet expose a top-k parameter)"
        );
    }
    // Use build_tree directly when flags deviate from defaults, so max_children
    // and label are honoured.  emit_tree_html + write are equivalent to
    // write_tree_html but allow us to pass the extra parameters.
    let tree_data = graphify_html::tree::build_tree(&g, Some(root_path), max_children, label);
    let title_name = tree_data
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("graph");
    let title = format!("{title_name} \u{2014} graphify tree viewer");
    let header = format!("{title_name} \u{2014} Knowledge Graph");
    let html = graphify_html::tree::emit_tree_html(&tree_data, &title, &header, 6000, 8000);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, html.as_bytes())?;
    eprintln!("wrote {}", out.display());
    Ok(())
}
