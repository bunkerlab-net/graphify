//! Parity tests against `graphify-py/tests/test_security.py`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use graphify_security::{
    MAX_FETCH_BYTES, MAX_TEXT_BYTES, SecurityError, safe_fetch, sanitize_label, test_support,
    validate_graph_path, validate_url,
};

// ---------------------------------------------------------------------------
// validate_url
// ---------------------------------------------------------------------------

#[test]
fn validate_url_accepts_http() {
    let url = validate_url("http://example.com/page").expect("valid http");
    assert_eq!(url, "http://example.com/page");
}

#[test]
fn validate_url_accepts_https() {
    let url = validate_url("https://arxiv.org/abs/1706.03762").expect("valid https");
    assert_eq!(url, "https://arxiv.org/abs/1706.03762");
}

#[test]
fn validate_url_rejects_file() {
    let err = validate_url("file:///etc/passwd").expect_err("file should be rejected");
    assert!(matches!(err, SecurityError::BlockedScheme { .. }));
    assert!(format!("{err}").contains("file"));
}

#[test]
fn validate_url_rejects_ftp() {
    let err = validate_url("ftp://files.example.com/data.zip").expect_err("ftp should be rejected");
    assert!(matches!(err, SecurityError::BlockedScheme { .. }));
    assert!(format!("{err}").contains("ftp"));
}

#[test]
fn validate_url_rejects_data() {
    let err = validate_url("data:text/html,<script>alert(1)</script>")
        .expect_err("data should be rejected");
    assert!(matches!(err, SecurityError::BlockedScheme { .. }));
    assert!(format!("{err}").contains("data"));
}

#[test]
fn validate_url_rejects_empty_scheme() {
    assert!(validate_url("//no-scheme.example.com").is_err());
}

#[test]
fn validate_url_rejects_private_ipv4_literal() {
    let err = validate_url("http://10.0.0.5/").expect_err("private IPv4 should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

#[test]
fn validate_url_rejects_loopback_ipv4_literal() {
    let err = validate_url("http://127.0.0.1/").expect_err("loopback should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

#[test]
fn validate_url_rejects_link_local_ipv4_literal() {
    let err = validate_url("http://169.254.169.254/").expect_err("link-local should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

#[test]
fn validate_url_rejects_cgn_ipv4_literal() {
    let err = validate_url("http://100.64.1.2/").expect_err("CGN range should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

#[test]
fn validate_url_rejects_metadata_host() {
    let err = validate_url("http://metadata.google.internal/")
        .expect_err("metadata host should be blocked");
    assert!(matches!(err, SecurityError::BlockedMetadataHost { .. }));
}

#[test]
fn validate_url_rejects_ipv6_loopback() {
    let err = validate_url("http://[::1]/").expect_err("ipv6 loopback should be blocked");
    assert!(matches!(err, SecurityError::BlockedPrivateIp { .. }));
}

// ---------------------------------------------------------------------------
// safe_fetch (mocked network)
// ---------------------------------------------------------------------------

#[test]
fn safe_fetch_rejects_file_url() {
    let err = safe_fetch(
        "file:///etc/passwd",
        MAX_FETCH_BYTES,
        Duration::from_secs(5),
    )
    .expect_err("file should be rejected");
    assert!(format!("{err}").contains("file"));
}

#[test]
fn safe_fetch_rejects_ftp_url() {
    let err = safe_fetch(
        "ftp://example.com/file.zip",
        MAX_FETCH_BYTES,
        Duration::from_secs(5),
    )
    .expect_err("ftp should be rejected");
    assert!(format!("{err}").contains("ftp"));
}

#[test]
fn net_safe_fetch_returns_bytes() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .with_status(200)
        .with_body("hello world")
        .create();
    let url = server.url();
    let result = test_support::fetch_allow_private(&url, MAX_FETCH_BYTES, Duration::from_secs(5))
        .expect("fetch ok");
    assert_eq!(result, b"hello world");
    mock.assert();
}

#[test]
fn net_safe_fetch_raises_on_non_2xx() {
    let mut server = mockito::Server::new();
    let _mock = server.mock("GET", "/missing").with_status(404).create();
    let url = format!("{}/missing", server.url());
    let err = test_support::fetch_allow_private(&url, MAX_FETCH_BYTES, Duration::from_secs(5))
        .expect_err("404 should fail");
    assert!(matches!(err, SecurityError::HttpStatus { status: 404, .. }));
}

#[test]
fn net_safe_fetch_raises_on_size_exceeded() {
    let big = "x".repeat(70_000);
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/huge")
        .with_status(200)
        .with_body(big)
        .create();
    let url = format!("{}/huge", server.url());
    let err = test_support::fetch_allow_private(&url, 65_536, Duration::from_secs(5))
        .expect_err("size limit should be enforced");
    assert!(matches!(err, SecurityError::SizeLimitExceeded { .. }));
}

#[test]
fn net_safe_fetch_text_decodes_utf8() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .with_status(200)
        .with_body("héllo wörld".as_bytes())
        .create();
    let url = server.url();
    let text = test_support::fetch_text_allow_private(&url, MAX_TEXT_BYTES, Duration::from_secs(5))
        .expect("fetch ok");
    assert_eq!(text, "héllo wörld");
}

#[test]
fn net_safe_fetch_text_replaces_bad_bytes() {
    let bad: &[u8] = b"hello \xff world";
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .with_status(200)
        .with_body(bad)
        .create();
    let url = server.url();
    let text = test_support::fetch_text_allow_private(&url, MAX_TEXT_BYTES, Duration::from_secs(5))
        .expect("fetch ok");
    assert!(text.contains("hello"));
    assert!(text.contains("world"));
    assert!(!text.contains('\u{ff}'));
}

// ---------------------------------------------------------------------------
// validate_graph_path
// ---------------------------------------------------------------------------

#[test]
fn validate_graph_path_allows_inside_base() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    std::fs::create_dir(&base).expect("mkdir");
    let graph = base.join("graph.json");
    std::fs::write(&graph, "{}").expect("write");
    let result = validate_graph_path(&graph, Some(&base)).expect("inside base should be allowed");
    assert_eq!(result, graph.canonicalize().expect("canonicalize"));
}

#[test]
fn validate_graph_path_blocks_traversal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    std::fs::create_dir(&base).expect("mkdir");
    let evil = base.join("..").join("etc_passwd");
    let err = validate_graph_path(&evil, Some(&base)).expect_err("traversal should be blocked");
    assert!(matches!(err, SecurityError::PathEscape { .. }));
}

#[test]
fn validate_graph_path_requires_base_exists() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    let err = validate_graph_path(base.join("graph.json"), Some(&base))
        .expect_err("missing base should fail");
    assert!(matches!(err, SecurityError::BaseMissing(_)));
}

#[test]
fn validate_graph_path_raises_if_file_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("graphify-out");
    std::fs::create_dir(&base).expect("mkdir");
    let err = validate_graph_path(base.join("missing.json"), Some(&base))
        .expect_err("missing file should fail");
    assert!(matches!(err, SecurityError::GraphFileMissing(_)));
}

// ---------------------------------------------------------------------------
// sanitize_label
// ---------------------------------------------------------------------------

#[test]
fn sanitize_label_passthrough_html_chars() {
    // sanitize_label does NOT HTML-escape — callers that inject into HTML
    // must wrap with htmlescape themselves.
    assert_eq!(sanitize_label(Some("<script>")), "<script>");
    assert_eq!(sanitize_label(Some("foo & bar")), "foo & bar");
}

#[test]
fn sanitize_label_strips_control_chars() {
    let result = sanitize_label(Some("hello\x00\x1fworld"));
    assert!(!result.contains('\x00'));
    assert!(!result.contains('\x1f'));
    assert!(result.contains("helloworld"));
}

#[test]
fn sanitize_label_caps_at_256() {
    let long = "a".repeat(300);
    let result = sanitize_label(Some(&long));
    assert!(result.chars().count() <= 256);
}

#[test]
fn sanitize_label_safe_passthrough() {
    assert_eq!(sanitize_label(Some("MyClass")), "MyClass");
    assert_eq!(sanitize_label(Some("extract_python")), "extract_python");
}

#[test]
fn sanitize_label_none_returns_empty() {
    assert_eq!(sanitize_label(None), "");
}
