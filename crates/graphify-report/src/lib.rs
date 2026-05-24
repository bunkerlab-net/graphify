//! Markdown report renderer for Graphify analysis results.
//!
//! Ports `graphify-py/graphify/report.py`. Accepts a [`graphify_build::Graph`]
//! and a JSON analysis object produced by `graphify-analyze`, and renders a
//! `GRAPH_REPORT.md` document covering communities, god-nodes, surprising
//! cross-file connections, hyperedges, knowledge gaps, token usage, detection
//! results, and suggested questions.
//!
//! Public entry points are [`render_report`] and [`write_report`].
//!
//! # Analysis value shape
//!
//! `analysis` must be a JSON object with the following fields (all
//! optional fields default gracefully):
//!
//! ```json
//! {
//!   "communities":           { "<cid>": ["node_id", ...], ... },
//!   "cohesion_scores":       { "<cid>": 0.75, ... },
//!   "community_labels":      { "<cid>": "Label", ... },
//!   "god_nodes":             [{ "id": "...", "label": "...", "degree": 5 }, ...],
//!   "surprising_connections":[{ "source": "...", "target": "...", ... }, ...],
//!   "detection_result":      { "total_files": 4, "total_words": 62400, "warning": null },
//!   "token_cost":            { "input": 1200, "output": 340 },
//!   "root":                  "./project",
//!   "suggested_questions":   null,
//!   "min_community_size":    3,
//!   "built_at_commit":       null
//! }
//! ```

mod analysis;
mod error;
mod render;
mod sections;
mod util;

pub use error::ReportError;
pub use render::{render_report, write_report};

// Visible to sub-modules in `sections`.
pub(crate) use util::safe_community_name;
