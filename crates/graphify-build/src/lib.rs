//! Graph assembly from extraction dicts.
//!
//! Ports `graphify-py/graphify/build.py`. Provides a [`Graph`] type that
//! mirrors `NetworkX` `Graph` / `DiGraph` / `MultiGraph` / `MultiDiGraph`
//! semantics closely enough for byte-identical JSON round-trips.

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
pub use build_fn::{build, build_from_json};
pub use dedup_label::deduplicate_by_label;
pub use error::BuildError;
pub use global_ops::{prefix_graph_for_global, prune_repo_from_graph};
pub use graph::{Edge, Graph, GraphKind};
pub use normalize::{norm_source_file, normalize_id};
