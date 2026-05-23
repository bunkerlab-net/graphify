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
pub mod graph;
mod serve_fn;
pub mod server;
mod state;
pub mod tools;

pub use error::ServeError;
pub use serve_fn::serve;
pub use state::ReloadState;
