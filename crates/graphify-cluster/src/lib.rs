//! Community detection, cohesion scoring, and community ID remapping for
//! `graphify_build::Graph` values.
//!
//! Ports `graphify-py/graphify/cluster.py`.
//!
//! # Primary entry points
//!
//! - [`cluster`] — run community detection on a graph and return a
//!   `{community_id → [node_ids]}` mapping sorted by community size.
//! - [`cohesion_score`] — compute the edge-density ratio for a single community.
//! - [`score_all`] — efficiently compute cohesion scores for all communities in
//!   a single graph pass.
//! - [`remap_communities_to_previous`] — remap community IDs to maximise
//!   overlap with a prior assignment, useful for temporal stability.
//!
//! # Algorithm
//!
//! The Python reference attempts Leiden (graspologic) first and falls
//! back to `NetworkX`'s Louvain when graspologic is not installed. This
//! crate matches that priority: it ships Leiden via the `leiden-rs`
//! crate as the default partitioner (see `leiden.rs`) and a hand-rolled
//! Louvain (see `louvain.rs`) seeded at 42 as a backup. The backend can
//! be forced via `GRAPHIFY_CLUSTER_BACKEND=louvain` for debugging.
//!
//! Leiden is preferred because it avoids the Phase-1 flip-flop pathology
//! that surfaced on real-world ~16k-node graphs (where Louvain hit a
//! 100-pass safety cap on tightly-coupled bridge nodes). Leiden's
//! refinement phase guarantees connected sub-communities and its Fast
//! Local Move algorithm bounds Phase-1 work to O(moves).
//!
//! See `.claude/local/notes/module_cluster.md` for a full rationale.

mod cluster;
mod cohesion;
mod constants;
mod edge_list;
mod hubs;
mod leiden;
mod louvain;
mod remap;
mod splits;

pub use cluster::cluster;
pub use cohesion::{cohesion_score, score_all};
pub use remap::remap_communities_to_previous;
