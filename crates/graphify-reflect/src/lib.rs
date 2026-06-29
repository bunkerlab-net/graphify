//! Deterministic "work memory" reflection over `graphify-out/memory/`.
//!
//! `graphify reflect` reads the Q&A memory docs that `graphify save-result`
//! files back into the graph, aggregates their outcome signals (`useful` /
//! `dead_end` / `corrected`), and writes a single lessons artifact an agent can
//! load at the start of the next session:
//!
//! - **Preferred sources** — nodes corroborated by multiple `useful` answers.
//! - **Tentative** — nodes seen useful only once (not yet corroborated).
//! - **Contested** — nodes with both positive and negative signals; recency decides.
//! - **Known dead ends** — questions/sources marked `dead_end`.
//! - **Corrections** — answers the user corrected, and the right answer.
//!
//! Source nodes are scored, not counted: each citation contributes a signed,
//! time-decayed value (`useful` positive, `dead_end`/`corrected` negative, with
//! a half-life so a fresh dead end outweighs a months-old useful). A node is
//! only promoted to "preferred" once corroborated by enough distinct results.
//!
//! It is deterministic: no LLM, stable sort orders, byte-stable output for a
//! given input and a given `now`. Ports `graphify-py/graphify/reflect.py`.

mod aggregate;
mod graph;
mod parse;
mod render;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

pub use aggregate::{
    AggResult, Bucket, ContestedEntry, Correction, DeadEnd, OutcomeCounts, SourceEntry,
    aggregate_lessons,
};
pub use graph::{load_known_nodes, load_node_community};
pub use parse::{MemoryDoc, load_memory_docs, parse_memory_doc};
pub use render::render_lessons_md;

/// A signal's weight halves every 30 days by default.
pub const DEFAULT_HALF_LIFE_DAYS: f64 = 30.0;
/// Distinct `useful` results needed to promote a node to "preferred".
pub const DEFAULT_MIN_CORROBORATION: usize = 2;
/// Bucket label for docs with no resolvable community.
pub(crate) const UNCATEGORIZED: &str = "Uncategorized";

/// `true` if `out_path` exists and is at least as new as every input that feeds
/// it (the memory docs, and `graph.json` plus its `.graphify_analysis.json` /
/// `.graphify_labels.json` sidecars when a graph is used, #1470).
///
/// Lets `graphify reflect --if-stale` skip a redundant run. A missing output is
/// never fresh (it must be built). Mtime-based and best-effort.
#[must_use]
pub fn lessons_fresh(out_path: &Path, memory_dir: &Path, graphs: GraphPaths<'_>) -> bool {
    let Ok(out_mtime) = std::fs::metadata(out_path).and_then(|m| m.modified()) else {
        return false; // missing/unreadable -> must build
    };
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    if memory_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(memory_dir)
    {
        for path in entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
        {
            if let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                newest = newest.max(mtime);
            }
        }
    }
    // The graph and its sidecars all feed the grouped lessons doc, so any one of
    // them being newer than the output makes the doc stale (#1470).
    if let Some(graph) = graphs.graph {
        let analysis = graphs.analysis.map_or_else(
            || sibling(graph, ".graphify_analysis.json"),
            Path::to_path_buf,
        );
        let labels = graphs.labels.map_or_else(
            || sibling(graph, ".graphify_labels.json"),
            Path::to_path_buf,
        );
        for input in [graph.to_path_buf(), analysis, labels] {
            if let Ok(mtime) = std::fs::metadata(&input).and_then(|m| m.modified()) {
                newest = newest.max(mtime);
            }
        }
    }
    out_mtime >= newest
}

/// Optional graph artifacts that enable community grouping + the node-existence gate.
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphPaths<'a> {
    /// `graph.json` path; community grouping is disabled when `None`.
    pub graph: Option<&'a Path>,
    /// `.graphify_analysis.json` override; defaults to the graph's sibling.
    pub analysis: Option<&'a Path>,
    /// `.graphify_labels.json` override; defaults to the graph's sibling.
    pub labels: Option<&'a Path>,
}

/// Scan `memory_dir`, write the lessons doc to `out_path`, return (path, agg).
///
/// When `graphs.graph` is given, lessons are grouped by community and source
/// nodes no longer in the graph are dropped; otherwise the doc is a single flat
/// section.
///
/// # Errors
///
/// Returns [`std::io::Error`] if the output directory cannot be created or the
/// lessons file cannot be written.
pub fn reflect(
    memory_dir: &Path,
    out_path: &Path,
    graphs: GraphPaths<'_>,
    now: DateTime<Utc>,
    half_life_days: f64,
    min_corroboration: usize,
) -> std::io::Result<(PathBuf, AggResult)> {
    let docs = load_memory_docs(memory_dir);

    let mut node_community = None;
    let mut known_nodes = None;
    if let Some(graph) = graphs.graph {
        let analysis: PathBuf = graphs.analysis.map_or_else(
            || sibling(graph, ".graphify_analysis.json"),
            Path::to_path_buf,
        );
        let labels: PathBuf = graphs.labels.map_or_else(
            || sibling(graph, ".graphify_labels.json"),
            Path::to_path_buf,
        );
        node_community = load_node_community(graph, &analysis, &labels);
        known_nodes = load_known_nodes(graph);
    }

    let agg = aggregate_lessons(
        &docs,
        node_community.as_ref(),
        now,
        half_life_days,
        min_corroboration,
        known_nodes.as_ref(),
    );

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, render_lessons_md(&agg))?;
    Ok((out_path.to_path_buf(), agg))
}

/// A path sharing `base`'s parent directory but with a different filename.
fn sibling(base: &Path, name: &str) -> PathBuf {
    base.parent()
        .map_or_else(|| PathBuf::from(name), |p| p.join(name))
}
