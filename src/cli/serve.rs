//! `serve` command — MCP server over stdio or Streamable HTTP.

use anyhow::Result;

use crate::cli::args::ServeTransport;
use crate::cli::graphify_out_dir;

/// Aggregated arguments for [`cmd_serve`].
///
/// The HTTP-only fields are read solely by the `http`-feature `serve_http`; the
/// no-feature stub ignores them, so they are dead code in a default build.
#[cfg_attr(not(feature = "http"), allow(dead_code))]
pub(crate) struct ServeOptions<'a> {
    pub graph: Option<&'a std::path::Path>,
    pub transport: ServeTransport,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub path: String,
    pub json_response: bool,
    pub stateless: bool,
    pub session_timeout: f64,
}

/// Start the MCP server backed by `graph.json` on the selected transport.
///
/// `stdio` (the default) reads line-delimited JSON-RPC from stdin; `http` serves
/// the Streamable HTTP transport (requires the binary's `http` feature). Mirrors
/// Python's `serve` / `serve_http`.
pub(crate) fn cmd_serve(opts: ServeOptions<'_>) -> Result<()> {
    let default_path = graphify_out_dir().join("graph.json");
    let path = opts.graph.unwrap_or(default_path.as_path());
    let graph_path = path.to_string_lossy().into_owned();

    match opts.transport {
        ServeTransport::Stdio => {
            eprintln!(
                "serving MCP over stdio (graph={}, Ctrl-C to stop) ...",
                path.display()
            );
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(graphify_serve::serve(&graph_path))?;
            Ok(())
        }
        ServeTransport::Http => serve_http(&graph_path, opts),
    }
}

/// Serve the Streamable HTTP transport (compiled with the `http` feature).
#[cfg(feature = "http")]
fn serve_http(graph_path: &str, opts: ServeOptions<'_>) -> Result<()> {
    let http_opts = graphify_serve::HttpOptions {
        host: opts.host,
        port: opts.port,
        api_key: opts.api_key,
        path: opts.path,
        json_response: opts.json_response,
        stateless: opts.stateless,
        session_timeout: opts.session_timeout,
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(graphify_serve::serve_http(graph_path, http_opts))?;
    Ok(())
}

/// Stub for builds without the `http` feature: `--transport http` fails loudly.
#[cfg(not(feature = "http"))]
fn serve_http(_graph_path: &str, _opts: ServeOptions<'_>) -> Result<()> {
    anyhow::bail!(
        "--transport http requires graphify built with the `http` feature \
         (e.g. `cargo install graphify --features http`)"
    )
}
