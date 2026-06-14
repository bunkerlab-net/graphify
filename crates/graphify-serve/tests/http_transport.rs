//! Integration tests for the Streamable HTTP transport (#1143).
//!
//! Exercise the in-process axum app (`build_app`) with a `tower` test client,
//! mirroring graphify-py's `_build_http_app` + ASGI-test-client approach. Only
//! compiled with the `http` feature.

#![cfg(feature = "http")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use graphify_serve::{HttpOptions, build_app};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Write a minimal graph and return its path as a string.
fn write_graph(dir: &std::path::Path) -> String {
    let graph = json!({
        "nodes": [
            {"id": "n1", "label": "alpha", "source_file": "a.py", "community": 0},
            {"id": "n2", "label": "beta", "source_file": "b.py", "community": 0},
        ],
        "edges": [
            {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    });
    let path = dir.join("graph.json");
    fs::write(&path, serde_json::to_string(&graph).expect("serialize")).expect("write graph");
    path.to_string_lossy().into_owned()
}

fn opts(json_response: bool, api_key: Option<&str>) -> HttpOptions {
    HttpOptions {
        json_response,
        api_key: api_key.map(str::to_string),
        ..HttpOptions::default()
    }
}

fn post(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn tools_list_returns_json_in_json_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let app = build_app(&gp, &opts(true, None)).expect("build_app");

    let resp = app
        .oneshot(post(
            "/mcp",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: Value = serde_json::from_str(&body_string(resp).await).expect("json");
    assert_eq!(v["id"], 1);
    assert!(
        v["result"]["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty())
    );
}

#[tokio::test]
async fn default_mode_streams_sse() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let app = build_app(&gp, &opts(false, None)).expect("build_app");

    let resp = app
        .oneshot(post(
            "/mcp",
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#,
        ))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    let body = body_string(resp).await;
    assert!(body.starts_with("event: message\ndata: "), "{body}");
    assert!(body.contains("\"id\":7"));
}

#[tokio::test]
async fn initialize_mints_session_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let app = build_app(&gp, &opts(true, None)).expect("build_app");

    let resp = app
        .oneshot(post(
            "/mcp",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        ))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers().get("mcp-session-id").is_some(),
        "initialize must mint a session id"
    );
}

#[tokio::test]
async fn stateless_mode_omits_session_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let mut o = opts(true, None);
    o.stateless = true;
    let app = build_app(&gp, &o).expect("build_app");

    let resp = app
        .oneshot(post(
            "/mcp",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        ))
        .await
        .expect("oneshot");
    assert!(resp.headers().get("mcp-session-id").is_none());
}

#[tokio::test]
async fn notification_returns_202_with_no_body() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let app = build_app(&gp, &opts(true, None)).expect("build_app");

    // No `id` => a notification, which gets no reply.
    let resp = app
        .oneshot(post(
            "/mcp",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        ))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    assert!(body_string(resp).await.is_empty());
}

#[tokio::test]
async fn missing_api_key_is_401() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let app = build_app(&gp, &opts(true, Some("s3cret"))).expect("build_app");

    let resp = app
        .oneshot(post(
            "/mcp",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bearer_and_x_api_key_are_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());

    for (name, value) in [
        (header::AUTHORIZATION.as_str(), "Bearer s3cret"),
        ("x-api-key", "s3cret"),
    ] {
        let app = build_app(&gp, &opts(true, Some("s3cret"))).expect("build_app");
        let req = Request::post("/mcp")
            .header(header::CONTENT_TYPE, "application/json")
            .header(name, value)
            .body(Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_string(),
            ))
            .expect("request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK, "{name} should authorize");
    }
}

#[tokio::test]
async fn wrong_api_key_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let app = build_app(&gp, &opts(true, Some("s3cret"))).expect("build_app");

    let req = Request::post("/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-api-key", "wrong")
        .body(Body::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_string(),
        ))
        .expect("request");
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_method_not_allowed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(dir.path());
    let app = build_app(&gp, &opts(true, None)).expect("build_app");

    let req = Request::get("/mcp").body(Body::empty()).expect("request");
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}
