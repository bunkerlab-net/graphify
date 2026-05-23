//! Community detection on `graphify_build::Graph` values.
//!
//! Ports `graphify-py/graphify/cluster.py`.
//!
//! ## Algorithm
//!
//! The Python reference attempts Leiden (graspologic) and falls back to
//! `NetworkX`'s Louvain. This crate ships a pure-Rust Louvain
//! implementation (see `louvain.rs`) seeded with `rand::rngs::StdRng` at
//! seed 42 — the same seed the Python fallback uses. Leiden is
//! intentionally **not** implemented; no suitable Rust crate exists in
//! the workspace and the structural-correctness tests do not require
//! identical community IDs.
//!
//! See `.claude/local/notes/module_cluster.md` for a full rationale.

mod cluster;
mod cohesion;
mod constants;
mod edge_list;
mod hubs;
mod louvain;
mod remap;
mod splits;

pub use cluster::cluster;
pub use cohesion::{cohesion_score, score_all};
pub use remap::remap_communities_to_previous;
