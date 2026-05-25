//! MCP stdio server — exposes graphify graph queries to LLM clients.
//!
//! Ports `graphify-py/graphify/serve.py`.
//!
//! # Modules
//!
//! - [`graph`]  — pure graph-query helpers (`_score_nodes`, `_bfs`, etc.)
//! - [`tools`]  — MCP tool handler implementations
//! - [`server`] — JSON-RPC stdio transport

mod error;
/// Pure graph-query helpers: scoring, BFS/DFS traversal, subgraph rendering.
pub mod graph;
mod serve_fn;
/// MCP stdio JSON-RPC server transport and message dispatcher.
pub mod server;
mod state;
/// MCP tool handler implementations invoked by the server dispatcher.
pub mod tools;

pub use error::ServeError;
pub use graph::query_terms;
pub use serve_fn::serve;
pub use state::ReloadState;
