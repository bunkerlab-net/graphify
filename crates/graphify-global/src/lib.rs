//! Cross-corpus graph merging.
//!
//! Ports `graphify-py/graphify/global_graph.py`. Combines multiple
//! per-repo graphs into a single global graph stored under
//! `~/.graphify/`.
//!
//! # Design
//!
//! The on-disk format is the `NetworkX` `node_link_data` JSON shape:
//! `{"directed": …, "multigraph": …, "graph": {}, "nodes": […], "links": […]}`.
//! Reading normalises `"links"` → `"edges"` so
//! [`graphify_build::build_from_json`] can parse it. Writing always
//! emits `"links"` for round-trip compatibility.
//!
//! [`graphify_build::prefix_graph_for_global`] and
//! [`graphify_build::prune_repo_from_graph`] are re-exported so callers
//! don't need a direct dependency on `graphify-build`.

mod error;
mod io;
mod manifest;
mod ops;
mod paths;

pub use error::GlobalError;
pub use graphify_build::{prefix_graph_for_global, prune_repo_from_graph};
pub use io::{file_hash, load_graph_from_file, save_graph_to_file};
pub use manifest::{Manifest, RepoEntry};
pub use ops::{AddSummary, global_add, global_list, global_remove};
pub use paths::{global_graph_path, global_manifest_path};
