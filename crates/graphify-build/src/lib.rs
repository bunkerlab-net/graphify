//! Graph assembly from extraction dictionaries.
//!
//! This crate ports `graphify-py/graphify/build.py`. Its primary entry
//! points are [`build`] (merge multiple extraction dicts into one graph)
//! and [`build_from_json`] (assemble a graph from a single extraction dict).
//!
//! The central data structure is [`Graph`], which mirrors the four
//! `NetworkX` graph kinds (`Graph` / `DiGraph` / `MultiGraph` /
//! `MultiDiGraph`) closely enough to produce byte-identical JSON
//! round-trips.
//!
//! # Pipeline overview
//!
//! 1. **Ingestion** — raw `file_type` values are coerced to the
//!    canonical set; `source` keys are renamed to `source_file`; node and
//!    edge IDs are normalised via [`normalize_id`].
//! 2. **Deduplication** — [`deduplicate_by_label`] merges nodes sharing a
//!    normalised label and rewrites edges to point at the surviving ID.
//! 3. **Global operations** — [`prefix_graph_for_global`] and
//!    [`prune_repo_from_graph`] prepare per-repo graphs for inclusion in
//!    or removal from a cross-corpus global graph.

mod attrs;
mod build_fn;
mod dedup_label;
mod error;
mod file_type;
mod global_ops;
mod graph;
mod ingest;
mod normalize;

pub use attrs::{EdgeAttrs, NodeAttrs};
pub use build_fn::{
    build, build_from_json, build_merge, build_merge_with_graph_cap, dedupe_edges, dedupe_nodes,
};
pub use dedup_label::{deduplicate_by_label, norm_label};
pub use error::BuildError;
pub use global_ops::{prefix_graph_for_global, prune_repo_from_graph};
pub use graph::{Edge, Graph, GraphKind};
pub use normalize::{norm_source_file, normalize_id};
