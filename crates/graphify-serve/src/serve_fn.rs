//! Public [`serve`] entry point — wires the JSON-RPC dispatcher to the
//! real stdio streams.

use crate::error::ServeError;
use crate::server;

/// Start the MCP server on the real stdio streams.
///
/// Loads the graph at `graph_path`, then enters the JSON-RPC dispatch
/// loop reading from stdin and writing to stdout until EOF.
///
/// # Errors
///
/// Returns [`ServeError`] if the graph file cannot be loaded.
pub async fn serve(graph_path: &str) -> Result<(), ServeError> {
    use tokio::io::{stdin, stdout};
    server::run_server(stdin(), stdout(), graph_path).await
}
