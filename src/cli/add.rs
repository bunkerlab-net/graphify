//! `add` command — fetch a URL and save it to ./raw, then update the graph.

use anyhow::Result;

/// Fetch a URL and save it to `./raw`, then update the graph.
///
/// Delegates to `graphify_ingest::ingest`, which handles both HTTP URLs and
/// local file paths. Mirrors `__main__.py`'s `add` command.
pub(crate) fn cmd_add(
    url: &str,
    author: Option<&str>,
    contributor: Option<&str>,
    dir: &std::path::Path,
) -> Result<()> {
    eprintln!("fetching {url} ...");
    let path = graphify_ingest::ingest(url, dir, author, contributor)?;
    println!("Saved to {}", path.display());
    println!("Run /graphify --update in your AI assistant to update the graph.");
    Ok(())
}
