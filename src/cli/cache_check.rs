//! `cache-check` command — check semantic cache for a list of files.

use anyhow::Result;

use crate::cli::graphify_out_dir;

/// Check semantic cache for a list of files and print results.
///
/// Mirrors `graphify cache-check <files_from> [--root <dir>]` at
/// `__main__.py:2919`. Reads one file path per line from `files_from`,
/// checks the semantic cache, writes the results to
/// `<root>/graphify-out/.graphify_cached.json` and
/// `<root>/graphify-out/.graphify_uncached.txt`, and prints a summary.
pub(crate) fn cmd_cache_check(files_from: &std::path::Path, root: &std::path::Path) -> Result<()> {
    let contents = std::fs::read_to_string(files_from)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", files_from.display()))?;
    let files: Vec<String> = contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let total = files.len();
    let split = graphify_cache::check_semantic_cache(&files, root);
    let hit_count = total - split.uncached_files.len();

    // Write results to the output dir, mirroring the Python behaviour.
    let out_dir = root.join(graphify_out_dir());
    std::fs::create_dir_all(&out_dir)?;

    if !split.cached_nodes.is_empty()
        || !split.cached_edges.is_empty()
        || !split.cached_hyperedges.is_empty()
    {
        let cached_json = serde_json::json!({
            "nodes": split.cached_nodes,
            "edges": split.cached_edges,
            "hyperedges": split.cached_hyperedges,
        });
        std::fs::write(
            out_dir.join(".graphify_cached.json"),
            serde_json::to_string(&cached_json)?,
        )?;
    }
    std::fs::write(
        out_dir.join(".graphify_uncached.txt"),
        split.uncached_files.join("\n"),
    )?;
    outln!(
        "Cache: {hit_count} hit, {} miss",
        split.uncached_files.len()
    );
    Ok(())
}
