//! `save-result` command — save a Q&A result to graphify-out/memory/.

use anyhow::Result;

/// Persist a Q&A result to `graphify-out/memory/` for the graph feedback loop.
///
/// Wraps `graphify_ingest::save_query_result` and prints the written path.
/// Mirrors Python's `save-result` command at `__main__.py`.
pub(crate) fn cmd_save_result(
    question: &str,
    answer: &str,
    query_type: &str,
    nodes: &[String],
    memory_dir: &std::path::Path,
    outcome: Option<&str>,
    correction: Option<&str>,
) -> Result<()> {
    let source_nodes = if nodes.is_empty() { None } else { Some(nodes) };
    let path = graphify_ingest::save_query_result(
        question,
        answer,
        memory_dir,
        query_type,
        source_nodes,
        outcome,
        correction,
    )?;
    outln!("Saved to {}", path.display());
    Ok(())
}
