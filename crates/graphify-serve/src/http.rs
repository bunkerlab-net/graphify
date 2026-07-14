//! Streamable HTTP transport for the MCP server (#1143, MCP spec 2025-03-26).
//!
//! Serves the same tools/resources as the stdio transport over a single
//! `POST <path>` endpoint, so one shared process can host the graph for a whole
//! team. graphify-py builds this on the `mcp` SDK's
//! `StreamableHTTPSessionManager` (starlette + uvicorn); the Rust port hand-rolls
//! the protocol on `axum`, reusing the same [`McpServerState`] dispatch path as
//! stdio. Gated behind the crate's `http` feature so the default build stays free
//! of the axum/tower/hyper tree.
//!
//! Divergence from graphify-py: graphify keeps no per-session state (the graph is
//! shared and reloads inside the tool handlers), so `--session-timeout` has
//! nothing to reap and is accepted as a no-op. A session id is still minted on
//! `initialize` (unless `--stateless`) so spec-conformant clients have one to
//! echo, but it is not validated on later requests.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::Value;

use crate::error::ServeError;
use crate::server::McpServerState;

/// Options for the Streamable HTTP transport (mirrors graphify-py `serve_http`).
pub struct HttpOptions {
    /// Bind host. `0.0.0.0`/`::`/empty exposes the server beyond localhost.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Optional API key required via `Authorization: Bearer` or `X-API-Key`.
    pub api_key: Option<String>,
    /// Mount path for the endpoint (default `/mcp`).
    pub path: String,
    /// Return a single `application/json` response instead of an SSE stream.
    pub json_response: bool,
    /// Run without minting per-session ids.
    pub stateless: bool,
    /// Accepted for parity; a no-op in Rust (no per-session state to reap).
    pub session_timeout: f64,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            api_key: None,
            path: "/mcp".to_string(),
            json_response: false,
            stateless: false,
            session_timeout: 3600.0,
        }
    }
}

/// Shared request context for the HTTP handlers.
struct HttpCtx {
    state: Mutex<McpServerState>,
    graph_path: String,
    api_key: Option<String>,
    json_response: bool,
    stateless: bool,
    session_counter: AtomicU64,
}

/// Start the MCP server over Streamable HTTP, blocking until the listener stops.
///
/// # Errors
///
/// Returns [`ServeError`] if the graph cannot be loaded, the address cannot be
/// bound, or the HTTP server exits with an error.
pub async fn serve_http(graph_path: &str, opts: HttpOptions) -> Result<(), ServeError> {
    // A blank key (`--api-key ""` or empty `GRAPHIFY_API_KEY`) must not be
    // mistaken for "auth on" — normalize it to None so the gate is unambiguous.
    let api_key = opts
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let app = build_app(graph_path, &opts)?;

    let auth_note = if api_key.is_some() {
        "api-key required"
    } else {
        "no auth (set --api-key to require one)"
    };
    eprintln!(
        "graphify MCP server (streamable-http) on http://{}:{}{} - {auth_note}",
        opts.host, opts.port, opts.path
    );
    if matches!(opts.host.as_str(), "0.0.0.0" | "::" | "") && api_key.is_none() {
        let shown = if opts.host.is_empty() {
            "0.0.0.0"
        } else {
            opts.host.as_str()
        };
        eprintln!(
            "WARNING: binding {shown} with no api-key exposes the graph unauthenticated \
             on the network. Set --api-key (or GRAPHIFY_API_KEY)."
        );
    }

    // An IPv6 literal host (e.g. `::1` or `::`) must be bracketed before the
    // `:port` suffix or the address parser splits on the wrong colon. An empty
    // host means "all interfaces" — bind `0.0.0.0` to match the warning above.
    let host = if opts.host.is_empty() {
        "0.0.0.0"
    } else {
        opts.host.as_str()
    };
    let addr = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{}", opts.port)
    } else {
        format!("{host}:{}", opts.port)
    };
    let listener = tokio::net::TcpListener::bind(addr.as_str())
        .await
        .map_err(|e| ServeError::Io(format!("could not bind {addr}: {e}")))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| ServeError::Io(e.to_string()))?;
    Ok(())
}

/// Build the axum app for the Streamable HTTP transport.
///
/// Split out from [`serve_http`] (which blocks on the listener) so the wiring can
/// be exercised in-process by a `tower` test client, mirroring graphify-py's
/// `_build_http_app`.
///
/// # Errors
///
/// Returns [`ServeError`] if the graph at `graph_path` cannot be loaded.
pub fn build_app(graph_path: &str, opts: &HttpOptions) -> Result<Router, ServeError> {
    // axum's `Router::route` panics on a path that doesn't start with `/`.
    // `opts.path` comes from a CLI flag, so validate it here and surface a
    // clean error rather than letting a bad value abort the process.
    if !opts.path.starts_with('/') {
        return Err(ServeError::InvalidHttpPath(opts.path.clone()));
    }
    let api_key = opts
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let ctx = Arc::new(HttpCtx {
        state: Mutex::new(McpServerState::load(graph_path)),
        graph_path: graph_path.to_string(),
        api_key,
        json_response: opts.json_response,
        stateless: opts.stateless,
        session_counter: AtomicU64::new(1),
    });
    Ok(build_router(&ctx, &opts.path))
}

/// Build the axum router mounting the MCP endpoint at `path`.
///
/// `POST` carries JSON-RPC; `DELETE` terminates a session (a no-op 200 here);
/// any other method on the path falls through to axum's 405.
fn build_router(ctx: &Arc<HttpCtx>, path: &str) -> Router {
    Router::new()
        .route(path, post(mcp_post).delete(mcp_delete))
        .with_state(Arc::clone(ctx))
}

/// Handle a `POST` JSON-RPC message.
async fn mcp_post(State(ctx): State<Arc<HttpCtx>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(expected) = &ctx.api_key
        && !api_key_ok(&headers, expected)
    {
        return unauthorized();
    }

    let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "invalid JSON-RPC body").into_response();
    };

    let is_initialize = msg.get("method").and_then(Value::as_str) == Some("initialize");
    // `handle` runs synchronous graph I/O under the state mutex; dispatch it on
    // the blocking pool so a slow or large graph load never stalls the async
    // executor. The mutex still serialises concurrent requests — which also
    // coordinates cache misses down to a single load — and that is acceptable for
    // this off-by-default, low-QPS HTTP transport.
    let ctx_for_handle = Arc::clone(&ctx);
    let Ok(response) = tokio::task::spawn_blocking(move || {
        // Recover from a poisoned lock rather than panicking: a prior panic in a
        // handler must not take the whole server down.
        let mut state = ctx_for_handle
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.handle(&msg, &ctx_for_handle.graph_path)
    })
    .await
    else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "handler task failed").into_response();
    };

    let Some(resp) = response else {
        // Notifications and responses get no body (JSON-RPC: no reply).
        return StatusCode::ACCEPTED.into_response();
    };

    let session_header = (is_initialize && !ctx.stateless).then(|| {
        let sid = ctx.session_counter.fetch_add(1, Ordering::Relaxed);
        format!("graphify-{sid:016x}")
    });

    if ctx.json_response {
        json_response(&resp, session_header.as_deref())
    } else {
        sse_response(&resp, session_header.as_deref())
    }
}

/// Handle a `DELETE` (session termination). graphify holds no per-session state,
/// so this acknowledges with 200 after the API-key gate.
async fn mcp_delete(State(ctx): State<Arc<HttpCtx>>, headers: HeaderMap) -> Response {
    if let Some(expected) = &ctx.api_key
        && !api_key_ok(&headers, expected)
    {
        return unauthorized();
    }
    StatusCode::OK.into_response()
}

/// Build the `Mcp-Session-Id` header pair when a session id was minted.
fn with_session(mut resp: Response, session: Option<&str>) -> Response {
    if let Some(sid) = session
        && let Ok(value) = HeaderValue::from_str(sid)
    {
        resp.headers_mut().insert("mcp-session-id", value);
    }
    resp
}

/// `application/json` single-response body.
fn json_response(resp: &Value, session: Option<&str>) -> Response {
    let body = serde_json::to_vec(resp).unwrap_or_default();
    let out = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response();
    with_session(out, session)
}

/// `text/event-stream` body carrying one `message` event with the response.
fn sse_response(resp: &Value, session: Option<&str>) -> Response {
    let json = serde_json::to_string(resp).unwrap_or_default();
    let body = format!("event: message\ndata: {json}\n\n");
    let out = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
        .into_response();
    with_session(out, session)
}

/// 401 with a JSON error body (mirrors graphify-py's `_ApiKeyMiddleware`).
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error": "unauthorized"}"#,
    )
        .into_response()
}

/// True when the request carries the expected key via `X-API-Key` or
/// `Authorization: Bearer <key>`. The comparison is constant-time.
fn api_key_ok(headers: &HeaderMap, expected: &str) -> bool {
    let provided = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
            let (scheme, token) = auth.split_once(' ')?;
            (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty())
                .then(|| token.trim().to_string())
        });
    provided.is_some_and(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
}

/// Length-checked constant-time byte comparison (mirrors Python
/// `hmac.compare_digest`, which also short-circuits on a length mismatch).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}
