//! `benchmark` command — measure token reduction vs naive full-corpus approach.

use anyhow::Result;

use crate::cli::graphify_out_dir;

/// Measure token reduction versus the naive full-corpus approach.
///
/// Loads `graph.json`, runs `graphify_benchmark::run_benchmark`, and prints
/// the formatted result to stdout. Mirrors Python `__main__.py`'s `benchmark` command.
pub(crate) fn cmd_benchmark(graph: Option<&std::path::Path>) -> Result<()> {
    let default_path = graphify_out_dir().join("graph.json");
    let path = graph.unwrap_or(default_path.as_path());
    eprintln!("benchmarking against {} ...", path.display());
    let start = std::time::Instant::now();
    let result = graphify_benchmark::run_benchmark(path, None, None)?;
    eprintln!("done in {:.1}s", start.elapsed().as_secs_f64());
    println!("{}", graphify_benchmark::format_benchmark(result.as_ref()));
    Ok(())
}
