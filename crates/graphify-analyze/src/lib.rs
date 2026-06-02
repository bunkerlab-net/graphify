//! Graph analysis for the Graphify pipeline.
//!
//! Ports `graphify-py/graphify/analyze.py`.
//!
//! This crate provides a collection of graph-level insight functions that
//! operate on a [`graphify_build::Graph`] snapshot produced by the extraction
//! and build stages:
//!
//! - **Centrality** (`centrality`): degree, betweenness, and edge-betweenness
//!   algorithms used internally by the higher-level analysis functions.
//! - **Classification** (`classify`): node-type predicates (`is_concept_node`,
//!   `is_json_key_node`) and the `file_category` helper, shared across modules.
//! - **Cross-language detection** (`cross_lang`): identifies edges that span
//!   different programming-language families or community boundaries.
//! - **Import cycles** (`cycles`): collapses the graph to file granularity and
//!   reports circular import dependencies via [`find_import_cycles`].
//! - **Diffing** (`diff`): compares two graph snapshots and surfaces added /
//!   removed nodes and edges via [`graph_diff`].
//! - **God-node detection** (`god_nodes`): returns the top-N highest-degree
//!   real entities via [`god_nodes`], filtering out file hubs and noise nodes.
//! - **Question suggestions** (`suggest`): generates LLM-ready prompts from
//!   AMBIGUOUS edges, bridge nodes, inferred relationships, isolated nodes,
//!   and low-cohesion communities via [`suggest_questions`].
//! - **Surprising connections** (`surprises`): scores and ranks cross-file or
//!   cross-community edges by how unexpected they are via [`surprising_connections`].

pub(crate) mod centrality;
pub(crate) mod classify;
pub(crate) mod cross_lang;
pub(crate) mod cycles;
pub(crate) mod diff;
pub(crate) mod god_nodes;
pub(crate) mod suggest;
pub(crate) mod surprises;

pub use classify::{file_category, is_concept_node, is_json_key_node};
pub use cycles::{ImportCycle, find_import_cycles, find_import_cycles_bounded};
pub use diff::graph_diff;
pub use god_nodes::god_nodes;
pub use suggest::suggest_questions;
pub use surprises::{SurpriseScoreInput, surprise_score, surprising_connections};
