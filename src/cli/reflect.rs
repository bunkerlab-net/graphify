//! `reflect` command — aggregate `graphify-out/memory/` outcomes into a
//! deterministic lessons doc (`graphify-out/reflections/LESSONS.md`).
//!
//! Mirrors Python's `reflect` command at `__main__.py`. Output directory is
//! honoured via `GRAPHIFY_OUT` (see [`crate::cli::graphify_out_dir`]).

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::graphify_out_dir;

/// Parsed `graphify reflect` arguments.
pub(crate) struct ReflectArgs {
    /// Memory directory; defaults to `<GRAPHIFY_OUT>/memory`.
    pub memory_dir: Option<PathBuf>,
    /// Output lessons path; defaults to `<GRAPHIFY_OUT>/reflections/LESSONS.md`.
    pub out: Option<PathBuf>,
    /// `graph.json`; auto-detected under `<GRAPHIFY_OUT>` when absent.
    pub graph: Option<PathBuf>,
    /// `.graphify_analysis.json` override.
    pub analysis: Option<PathBuf>,
    /// `.graphify_labels.json` override.
    pub labels: Option<PathBuf>,
    /// Time-decay half-life in days.
    pub half_life_days: f64,
    /// Distinct useful results to promote a node to preferred.
    pub min_corroboration: usize,
    /// Skip the rebuild when `LESSONS.md` is already current.
    pub if_stale: bool,
}

/// Run `graphify reflect`, writing the lessons doc and printing a summary.
///
/// # Errors
///
/// Returns an error if the lessons file cannot be written.
pub(crate) fn cmd_reflect(args: ReflectArgs) -> Result<()> {
    let out_dir = graphify_out_dir();
    let memory_dir = args.memory_dir.unwrap_or_else(|| out_dir.join("memory"));
    let out_path = args
        .out
        .unwrap_or_else(|| out_dir.join("reflections").join("LESSONS.md"));

    // Auto-detect graph.json under the output dir when --graph is not given, so
    // lessons are grouped by community without the user wiring it up.
    let graph = args.graph.or_else(|| {
        let default_graph = out_dir.join("graph.json");
        default_graph.exists().then_some(default_graph)
    });

    let graphs = graphify_reflect::GraphPaths {
        graph: graph.as_deref(),
        analysis: args.analysis.as_deref(),
        labels: args.labels.as_deref(),
    };

    if args.if_stale && graphify_reflect::lessons_fresh(&out_path, &memory_dir, graphs) {
        outln!(
            "Lessons already up to date -> {} (skipped; omit --if-stale to force)",
            out_path.display()
        );
        return Ok(());
    }

    let (path, agg) = graphify_reflect::reflect(
        &memory_dir,
        &out_path,
        graphs,
        chrono::Utc::now(),
        args.half_life_days,
        args.min_corroboration,
    )?;
    outln!(
        "Reflected {} memories ({} useful, {} dead ends, {} corrected) -> {}",
        agg.total,
        agg.counts.useful,
        agg.counts.dead_end,
        agg.counts.corrected,
        path.display()
    );
    Ok(())
}
