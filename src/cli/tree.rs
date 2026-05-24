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
    // Python uses the longest common directory of every node's `source_file`
    // when `--root` is omitted (see `tree_html._common_root`). The Rust
    // `build_tree` already derives that root internally when `root` is `None`,
    // so forward `None` straight through instead of falling back to CWD.
    let default_output = graph_path.with_file_name("GRAPH_TREE.html");
    let out = output.unwrap_or(default_output.as_path());
    eprintln!("rendering tree HTML for {} nodes ...", g.node_count());
    let _ = top_k_edges; // accepted for CLI compatibility; Python ignores it too.
    let tree_data = graphify_html::tree::build_tree(&g, root, max_children, label);
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
    // Match Python's stdout output: wrote line + "open with:" hint.
    // `as f64` on a u64 file size is fine: 52-bit mantissa overflows only
    // beyond ~4 PB which a tree HTML page will never hit.
    #[allow(clippy::cast_precision_loss)]
    let size_kb = out.metadata().map_or(0.0, |m| m.len() as f64 / 1024.0);
    let abs = out.canonicalize().unwrap_or_else(|_| out.to_path_buf());
    println!("wrote {} ({size_kb:.1} KB)", out.display());
    println!(
        "open with: xdg-open {}  (or file://{})",
        out.display(),
        abs.display()
    );
    Ok(())
}
