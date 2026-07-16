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
/// Streamable HTTP transport (MCP spec 2025-03-26); requires the `http` feature.
#[cfg(feature = "http")]
pub mod http;
/// Opt-in append-only query logging (off by default; enable via
/// `GRAPHIFY_QUERY_LOG`/`GRAPHIFY_QUERY_LOG_ENABLE`, #1797).
pub mod querylog;
mod serve_fn;
/// MCP stdio JSON-RPC server transport and message dispatcher.
pub mod server;
mod state;
/// MCP tool handler implementations invoked by the server dispatcher.
pub mod tools;

pub use error::ServeError;
pub use graph::query_terms;
#[cfg(feature = "http")]
pub use http::{HttpOptions, build_app, serve_http};
pub use querylog::{QueryLog, log_query, nodes_from_result};
pub use serve_fn::serve;
pub use state::ReloadState;
