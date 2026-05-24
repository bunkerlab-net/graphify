//! Tuning constants for community detection.
//!
//! Each value mirrors the matching tunable in the Python reference so
//! cluster outputs match across implementations within statistical noise.

/// Communities larger than this fraction of graph nodes get split.
pub(crate) const MAX_COMMUNITY_FRACTION: f64 = 0.25;
/// Only split a community if it has at least this many nodes.
pub(crate) const MIN_SPLIT_SIZE: usize = 10;
/// Re-split communities with cohesion below this threshold.
pub(crate) const COHESION_SPLIT_THRESHOLD: f64 = 0.05;
/// Only apply cohesion split to communities with at least this many
/// nodes.
pub(crate) const COHESION_SPLIT_MIN_SIZE: usize = 50;
